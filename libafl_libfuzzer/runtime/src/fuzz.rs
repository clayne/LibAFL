use core::ffi::c_int;
#[cfg(unix)]
use std::{
    fmt::Debug,
    fs::File,
    io::{Write, stderr, stdout},
    net::TcpListener,
    os::fd::AsRawFd,
    str::FromStr,
};

#[cfg(feature = "tui_monitor")]
use libafl::monitors::tui::TuiMonitor;
use libafl::{
    Error, Fuzzer, HasMetadata,
    corpus::Corpus,
    events::{EventReceiver, ProgressReporter, SimpleEventManager},
    executors::ExitKind,
    monitors::MultiMonitor,
    stages::StagesTuple,
    state::{HasCurrentStageId, HasExecutions, HasLastReportTime, HasSolutions, Stoppable},
};
#[cfg(unix)]
use libafl::{
    events::{EventConfig, SimpleRestartingEventManager, launcher::Launcher},
    monitors::Monitor,
};
#[cfg(unix)]
use libafl_bolts::{
    core_affinity::Cores,
    shmem::{ShMemProvider, StdShMemProvider},
};

use crate::{feedbacks::LibfuzzerCrashCauseMetadata, fuzz_with, options::LibfuzzerOptions};

#[cfg(unix)]
fn destroy_output_fds(options: &LibfuzzerOptions) {
    // TODO: this could probably use libafl_bolts::os::dup_and_mute_outputsputs instead.

    use libafl_bolts::os::{dup2, null_fd};

    let null_fd = null_fd().unwrap();
    let stdout_fd = stdout().as_raw_fd();
    let stderr_fd = stderr().as_raw_fd();

    #[cfg(feature = "tui_monitor")]
    if options.tui() {
        // # Safety
        // The file descriptors schould still be valid at this point.
        unsafe {
            dup2(null_fd, stdout_fd).unwrap();
            dup2(null_fd, stderr_fd).unwrap();
        }
        return;
    }

    if options.close_fd_mask() != 0 {
        if options.close_fd_mask() & u8::try_from(stderr_fd).unwrap() != 0 {
            // # Safety
            // The file descriptors schould still be valid at this point.
            unsafe {
                dup2(null_fd, stdout_fd).unwrap();
            }
        }
        if options.close_fd_mask() & u8::try_from(stderr_fd).unwrap() != 0 {
            // # Safety
            // The file descriptors schould still be valid at this point.
            unsafe {
                dup2(null_fd, stderr_fd).unwrap();
            }
        }
    }
}

fn do_fuzz<F, ST, E, I, S, EM>(
    options: &LibfuzzerOptions,
    fuzzer: &mut F,
    stages: &mut ST,
    executor: &mut E,
    state: &mut S,
    mgr: &mut EM,
) -> Result<(), Error>
where
    F: Fuzzer<E, EM, I, S, ST>,
    S: HasMetadata
        + HasExecutions
        + HasSolutions<I>
        + HasLastReportTime
        + HasCurrentStageId
        + Stoppable,
    EM: ProgressReporter<S> + EventReceiver<I, S>,
    ST: StagesTuple<E, EM, S, F>,
{
    if let Some(solution) = state.solutions().last() {
        let kind = state
            .solutions()
            .get(solution)
            .expect("Last solution was not available")
            .borrow()
            .metadata::<LibfuzzerCrashCauseMetadata>()
            .expect("Crash cause not attached to solution")
            .kind();
        let mut halt = false;
        match kind {
            ExitKind::Oom if !options.ignore_ooms() => halt = true,
            ExitKind::Crash if !options.ignore_crashes() => halt = true,
            ExitKind::Timeout if !options.ignore_timeouts() => halt = true,
            _ => {
                log::info!("Ignoring {kind:?} according to requested ignore rules.");
            }
        }
        if halt {
            log::info!("Halting; the error on the next line is actually okay. :)");
            return Err(Error::shutting_down());
        }
    }
    if options.runs() == 0 {
        fuzzer.fuzz_loop(stages, executor, state, mgr)?;
    } else {
        for _ in 0..options.runs() {
            fuzzer.fuzz_one(stages, executor, state, mgr)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn fuzz_single_forking<M>(
    options: &LibfuzzerOptions,
    harness: &extern "C" fn(*const u8, usize) -> c_int,
    mut shmem_provider: StdShMemProvider,
    monitor: M,
) -> Result<(), Error>
where
    M: Monitor + Debug,
{
    destroy_output_fds(options);
    fuzz_with!(options, harness, do_fuzz, |fuzz_single| {
        let (state, mgr): (
            Option<StdState<_, _, _, _>>,
            SimpleRestartingEventManager<_, _, StdState<_, _, _, _>, _, _>,
        ) = match SimpleRestartingEventManager::launch(monitor, &mut shmem_provider) {
            // The restarting state will spawn the same process again as child, then restarted it each time it crashes.
            Ok(res) => res,
            Err(err) => match err {
                Error::ShuttingDown => {
                    return Ok(());
                }
                _ => {
                    panic!("Failed to setup the restarter: {err}");
                }
            },
        };
        crate::start_fuzzing_single(fuzz_single, state, mgr)
    })
}

#[cfg(unix)]
fn fuzz_many_forking<M>(
    options: &LibfuzzerOptions,
    harness: &extern "C" fn(*const u8, usize) -> c_int,
    shmem_provider: StdShMemProvider,
    forks: usize,
    monitor: M,
) -> Result<(), Error>
where
    M: Monitor + Clone + Debug + 'static,
{
    // Communicate the selected port to subprocesses
    const PORT_PROVIDER_VAR: &str = "_LIBAFL_LIBFUZZER_FORK_PORT";

    destroy_output_fds(options);
    let broker_port = std::env::var(PORT_PROVIDER_VAR)
        .map_err(Error::from)
        .and_then(|s| u16::from_str(&s).map_err(Error::from))
        .or_else(|_| {
            TcpListener::bind("127.0.0.1:0").map(|sock| {
                let port = sock.local_addr().unwrap().port();
                unsafe {
                    std::env::set_var(PORT_PROVIDER_VAR, port.to_string());
                }
                port
            })
        })?;
    fuzz_with!(options, harness, do_fuzz, |mut run_client| {
        let cores = Cores::from((0..forks).collect::<Vec<_>>());

        match Launcher::builder()
            .shmem_provider(shmem_provider)
            .configuration(EventConfig::from_name(options.fuzzer_name()))
            .monitor(monitor)
            .run_client(&mut run_client)
            .cores(&cores)
            .broker_port(broker_port)
            // TODO .remote_broker_addr(opt.remote_broker_addr)
            .stdout_file(Some("/dev/null"))
            .build()
            .launch()
        {
            Ok(()) => (),
            Err(Error::ShuttingDown) => println!("Fuzzing stopped by user. Good bye."),
            res @ Err(_) => return res,
        }
        Ok(())
    })
}

fn create_monitor_closure() -> impl Fn(&str) + Clone {
    #[cfg(unix)]
    let stderr_fd =
        std::os::fd::RawFd::from_str(&std::env::var(crate::STDERR_FD_VAR).unwrap()).unwrap(); // set in main
    move |s| {
        #[cfg(unix)]
        {
            use std::os::fd::FromRawFd;

            // unfortunate requirement to meet Clone... thankfully, this does not
            // generate effectively any overhead (no allocations, calls get merged)
            let mut stderr = unsafe { File::from_raw_fd(stderr_fd) };
            writeln!(stderr, "{s}").expect("Could not write to stderr???");
            std::mem::forget(stderr); // do not close the descriptor!
        }
        #[cfg(not(unix))]
        eprintln!("{s}");
    }
}

pub fn fuzz(
    options: &LibfuzzerOptions,
    harness: &extern "C" fn(*const u8, usize) -> c_int,
) -> Result<(), Error> {
    #[cfg(unix)]
    if let Some(forks) = options.forks() {
        let shmem_provider = StdShMemProvider::new().expect("Failed to init shared memory");

        #[cfg(feature = "tui_monitor")]
        if options.tui() {
            let monitor = TuiMonitor::builder()
                .title(options.fuzzer_name())
                .enhanced_graphics(true)
                .build();
            return fuzz_many_forking(options, harness, shmem_provider, forks, monitor);
        }

        // Non-TUI path or when tui_monitor feature is disabled
        let monitor = MultiMonitor::new(create_monitor_closure());

        if forks == 1 {
            return fuzz_single_forking(options, harness, shmem_provider, monitor);
        }

        return fuzz_many_forking(options, harness, shmem_provider, forks, monitor);
    }

    #[cfg(feature = "tui_monitor")]
    if options.tui() {
        // if the user specifies TUI, we assume they want to fork; it would not be possible to use
        // TUI safely otherwise
        let shmem_provider = StdShMemProvider::new().expect("Failed to init shared memory");
        let monitor = TuiMonitor::builder()
            .title(options.fuzzer_name())
            .enhanced_graphics(true)
            .build();
        return fuzz_many_forking(options, harness, shmem_provider, 1, monitor);
    }

    // Default path when no forks or TUI are specified, or when tui_monitor feature is disabled
    #[cfg(unix)]
    destroy_output_fds(options);

    fuzz_with!(options, harness, do_fuzz, |fuzz_single| {
        let mgr = SimpleEventManager::new(MultiMonitor::new(create_monitor_closure()));
        crate::start_fuzzing_single(fuzz_single, None, mgr)
    })
}
