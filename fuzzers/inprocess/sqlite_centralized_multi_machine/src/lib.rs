//! A libfuzzer-like fuzzer with llmp-multithreading support and restarts
//! The example harness is built for libpng.
//! In this example, you will see the use of the `launcher` feature.
//! The `launcher` will spawn new processes for each cpu core.
use core::time::Duration;
use std::{env, net::SocketAddr, path::PathBuf, str::FromStr};

use clap::{self, Parser};
use libafl::{
    corpus::{Corpus, InMemoryCorpus, OnDiskCorpus},
    events::{
        centralized::CentralizedEventManager, launcher::CentralizedLauncher,
        multi_machine::NodeDescriptor, ClientDescription, EventConfig,
    },
    executors::{inprocess::InProcessExecutor, ExitKind},
    feedback_or, feedback_or_fast,
    feedbacks::{CrashFeedback, MaxMapFeedback, TimeFeedback, TimeoutFeedback},
    fuzzer::{Fuzzer, StdFuzzer},
    inputs::{BytesInput, HasTargetBytes},
    monitors::MultiMonitor,
    mutators::{
        havoc_mutations::havoc_mutations,
        scheduled::{tokens_mutations, HavocScheduledMutator},
        token_mutations::Tokens,
    },
    observers::{CanTrack, HitcountsMapObserver, TimeObserver},
    schedulers::{IndexesLenTimeMinimizerScheduler, QueueScheduler},
    stages::mutational::StdMutationalStage,
    state::{HasCorpus, StdState},
    Error, HasMetadata,
};
use libafl_bolts::{
    core_affinity::Cores,
    rands::StdRand,
    shmem::{ShMemProvider, StdShMemProvider},
    tuples::{tuple_list, Merge},
    AsSlice,
};
use libafl_targets::{libfuzzer_initialize, libfuzzer_test_one_input, std_edges_map_observer};
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Parse a millis string to a [`Duration`]. Used for arg parsing.
fn timeout_from_millis_str(time: &str) -> Result<Duration, Error> {
    Ok(Duration::from_millis(time.parse()?))
}

/// The commandline args this fuzzer accepts
#[derive(Debug, Parser)]
#[command(
    name = "libfuzzer_libpng_launcher",
    about = "A libfuzzer-like fuzzer for libpng with llmp-multithreading support and a launcher",
    author = "Andrea Fioraldi <andreafioraldi@gmail.com>, Dominik Maier <domenukk@gmail.com>"
)]
struct Opt {
    #[arg(
    short,
    long,
    value_parser = Cores::from_cmdline,
    help = "Spawn a client in each of the provided cores. Broker runs in the 0th core. 'all' to select all available cores. 'none' to run a client without binding to any core. eg: '1,2-4,6' selects the cores 1,2,3,4,6.",
    name = "CORES"
    )]
    cores: Cores,

    #[arg(
        short = 'p',
        long,
        help = "Choose the broker TCP port, default is 1337",
        name = "PORT",
        default_value = "1337"
    )]
    broker_port: u16,

    #[arg(short = 'a', long, help = "Specify a remote broker", name = "REMOTE")]
    remote_broker_addr: Option<SocketAddr>,

    #[arg(
        short,
        long,
        help = "Set an initial corpus directory",
        name = "INPUT",
        required = true
    )]
    input: Vec<PathBuf>,

    #[arg(
        short,
        long,
        help = "Set the output directory, default is ./out",
        name = "OUTPUT",
        default_value = "./out"
    )]
    output: PathBuf,

    #[arg(
    value_parser = timeout_from_millis_str,
    short,
    long,
    help = "Set the exeucution timeout in milliseconds, default is 10000",
    name = "TIMEOUT",
    default_value = "10000"
    )]
    timeout: Duration,

    /*
    /// This fuzzer has hard-coded tokens
    #[arg(

        short = "x",
        long,
        help = "Feed the fuzzer with an user-specified list of tokens (often called \"dictionary\"",
        name = "TOKENS",
        multiple = true
    )]
    tokens: Vec<PathBuf>,
    */
    #[arg(
        long,
        help = "The address of the parent node to connect to, if any",
        name = "PARENT_ADDR",
        default_value = None
    )]
    parent_addr: Option<String>,

    #[arg(
        long,
        help = "The port on which the node will listen on, if children are to be expected",
        name = "NODE_LISTENING_PORT",
        default_value = None
    )]
    node_listening_port: Option<u16>,
}

/// The main fn, `no_mangle` as it is a C symbol
#[no_mangle]
pub extern "C" fn libafl_main() {
    env_logger::init();

    // Registry the metadata types used in this fuzzer
    // Needed only on no_std
    // unsafe { RegistryBuilder::register::<Tokens>(); }
    let opt = Opt::parse();

    let broker_port = opt.broker_port;
    let cores = opt.cores;

    println!(
        "Workdir: {:?}",
        env::current_dir().unwrap().to_string_lossy().to_string()
    );

    let shmem_provider = StdShMemProvider::new().expect("Failed to init shared memory");

    let monitor = MultiMonitor::new(|s| println!("{s}"));

    let mut secondary_run_client =
        |state: Option<_>,
         mut mgr: CentralizedEventManager<_, _, _, _, _>,
         _client_description: ClientDescription| {
            // Create an observation channel using the coverage map
            let edges_observer =
                HitcountsMapObserver::new(unsafe { std_edges_map_observer("edges") })
                    .track_indices();

            // Create an observation channel to keep track of the execution time
            let time_observer = TimeObserver::new("time");

            // Feedback to rate the interestingness of an input
            // This one is composed by two Feedbacks in OR
            let mut feedback = feedback_or!(
                // New maximization map feedback linked to the edges observer and the feedback state
                MaxMapFeedback::new(&edges_observer),
                // Time feedback, this one does not need a feedback state
                TimeFeedback::new(&time_observer)
            );

            // A feedback to choose if an input is a solution or not
            let mut objective = feedback_or_fast!(CrashFeedback::new(), TimeoutFeedback::new());

            // If not restarting, create a State from scratch
            let mut state = state.unwrap_or_else(|| {
                StdState::new(
                    // RNG
                    StdRand::new(),
                    // Corpus that will be evolved, we keep it in memory for performance
                    InMemoryCorpus::new(),
                    // Corpus in which we store solutions (crashes in this example),
                    // on disk so the user can get them after stopping the fuzzer
                    OnDiskCorpus::new(&opt.output).unwrap(),
                    // States of the feedbacks.
                    // The feedbacks can report the data that should persist in the State.
                    &mut feedback,
                    // Same for objective feedbacks
                    &mut objective,
                )
                .unwrap()
            });

            println!("We're a client, let's fuzz :)");

            // Create a PNG dictionary if not existing
            if state.metadata_map().get::<Tokens>().is_none() {
                state.add_metadata(Tokens::from([
                    vec![137, 80, 78, 71, 13, 10, 26, 10], // PNG header
                    "IHDR".as_bytes().to_vec(),
                    "IDAT".as_bytes().to_vec(),
                    "PLTE".as_bytes().to_vec(),
                    "IEND".as_bytes().to_vec(),
                ]));
            }

            // Setup a basic mutator with a mutational stage
            let mutator = HavocScheduledMutator::new(havoc_mutations().merge(tokens_mutations()));
            let mut stages = tuple_list!(StdMutationalStage::new(mutator));

            // A minimization+queue policy to get testcasess from the corpus
            let scheduler =
                IndexesLenTimeMinimizerScheduler::new(&edges_observer, QueueScheduler::new());

            // A fuzzer with feedbacks and a corpus scheduler
            let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

            // The wrapped harness function, calling out to the LLVM-style harness
            let mut harness = |input: &BytesInput| {
                let target = input.target_bytes();
                let buf = target.as_slice();
                unsafe {
                    libfuzzer_test_one_input(buf);
                }
                ExitKind::Ok
            };

            let mut executor = InProcessExecutor::with_timeout(
                &mut harness,
                tuple_list!(edges_observer, time_observer),
                &mut fuzzer,
                &mut state,
                &mut mgr,
                opt.timeout,
            )?;

            // The actual target run starts here.
            // Call LLVMFUzzerInitialize() if present.
            let args: Vec<String> = env::args().collect();
            if unsafe { libfuzzer_initialize(&args) } == -1 {
                println!("Warning: LLVMFuzzerInitialize failed with -1");
            }

            // In case the corpus is empty (on first run), reset
            if state.must_load_initial_inputs() {
                state
                    .load_initial_inputs(&mut fuzzer, &mut executor, &mut mgr, &opt.input)
                    .unwrap_or_else(|_| {
                        panic!("Failed to load initial corpus at {:?}", &opt.input)
                    });
                println!("We imported {} inputs from disk.", state.corpus().count());
            }
            if !mgr.is_main() {
                fuzzer.fuzz_loop(&mut stages, &mut executor, &mut state, &mut mgr)?;
            } else {
                let mut empty_stages = tuple_list!();
                fuzzer.fuzz_loop(&mut empty_stages, &mut executor, &mut state, &mut mgr)?;
            }
            Ok(())
        };

    let mut main_run_client = secondary_run_client; // clone it just for borrow checker

    let parent_addr: Option<SocketAddr> = opt
        .parent_addr
        .map(|parent_str| SocketAddr::from_str(parent_str.as_str()).expect("Wrong parent address"));

    let mut node_description = NodeDescriptor::builder().parent_addr(parent_addr).build();

    if opt.node_listening_port.is_some() {
        node_description.node_listening_port = opt.node_listening_port;
    }

    match CentralizedLauncher::builder()
        .shmem_provider(shmem_provider)
        .configuration(EventConfig::from_name("default"))
        .monitor(monitor)
        .secondary_run_client(&mut secondary_run_client)
        .main_run_client(&mut main_run_client)
        .cores(&cores)
        .broker_port(broker_port)
        .centralized_broker_port(broker_port + 1)
        .remote_broker_addr(opt.remote_broker_addr)
        .multi_machine_node_descriptor(node_description)
        // .stdout_file(Some("/dev/null"))
        .build()
        .launch()
    {
        Ok(()) => (),
        Err(Error::ShuttingDown) => println!("Fuzzing stopped by user. Good bye."),
        Err(err) => panic!("Failed to run launcher: {err:?}"),
    }
}
