//! The queue corpus scheduler for power schedules.

use alloc::vec::Vec;
use core::{marker::PhantomData, time::Duration};

use libafl_bolts::{
    tuples::{Handle, Handled},
    Named,
};
use serde::{Deserialize, Serialize};

use crate::{
    corpus::{Corpus, CorpusId, HasTestcase, Testcase},
    inputs::UsesInput,
    observers::{MapObserver, ObserversTuple},
    schedulers::{AflScheduler, HasQueueCycles, RemovableScheduler, Scheduler},
    state::{HasCorpus, State, UsesState},
    Error, HasMetadata,
};

/// The n fuzz size
pub const N_FUZZ_SIZE: usize = 1 << 21;

libafl_bolts::impl_serdeany!(SchedulerMetadata);

/// The metadata used for power schedules
#[derive(Serialize, Deserialize, Clone, Debug)]
#[cfg_attr(
    any(not(feature = "serdeany_autoreg"), miri),
    allow(clippy::unsafe_derive_deserialize)
)] // for SerdeAny
pub struct SchedulerMetadata {
    /// Powerschedule strategy
    strat: Option<PowerSchedule>,
    /// Measured exec time during calibration
    exec_time: Duration,
    /// Calibration cycles
    cycles: u64,
    /// Size of the observer map
    bitmap_size: u64,
    /// Sum of `log(bitmap_size`)
    bitmap_size_log: f64,
    /// Number of filled map entries
    bitmap_entries: u64,
    /// Queue cycles
    queue_cycles: u64,
    /// The vector to contain the frequency of each execution path.
    n_fuzz: Vec<u32>,
}

/// The metadata for runs in the calibration stage.
impl SchedulerMetadata {
    /// Creates a new [`struct@SchedulerMetadata`]
    #[must_use]
    pub fn new(strat: Option<PowerSchedule>) -> Self {
        Self {
            strat,
            exec_time: Duration::from_millis(0),
            cycles: 0,
            bitmap_size: 0,
            bitmap_size_log: 0.0,
            bitmap_entries: 0,
            queue_cycles: 0,
            n_fuzz: vec![0; N_FUZZ_SIZE],
        }
    }

    /// The `PowerSchedule`
    #[must_use]
    pub fn strat(&self) -> Option<PowerSchedule> {
        self.strat
    }

    /// Set the `PowerSchedule`
    pub fn set_strat(&mut self, strat: Option<PowerSchedule>) {
        self.strat = strat;
    }

    /// The measured exec time during calibration
    #[must_use]
    pub fn exec_time(&self) -> Duration {
        self.exec_time
    }

    /// Set the measured exec
    pub fn set_exec_time(&mut self, time: Duration) {
        self.exec_time = time;
    }

    /// The cycles
    #[must_use]
    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Sets the cycles
    pub fn set_cycles(&mut self, val: u64) {
        self.cycles = val;
    }

    /// The bitmap size
    #[must_use]
    pub fn bitmap_size(&self) -> u64 {
        self.bitmap_size
    }

    /// Sets the bitmap size
    pub fn set_bitmap_size(&mut self, val: u64) {
        self.bitmap_size = val;
    }

    #[must_use]
    /// The sum of log(`bitmap_size`)
    pub fn bitmap_size_log(&self) -> f64 {
        self.bitmap_size_log
    }

    /// Setts the sum of log(`bitmap_size`)
    pub fn set_bitmap_size_log(&mut self, val: f64) {
        self.bitmap_size_log = val;
    }

    /// The number of filled map entries
    #[must_use]
    pub fn bitmap_entries(&self) -> u64 {
        self.bitmap_entries
    }

    /// Sets the number of filled map entries
    pub fn set_bitmap_entries(&mut self, val: u64) {
        self.bitmap_entries = val;
    }

    /// The amount of queue cycles
    #[must_use]
    pub fn queue_cycles(&self) -> u64 {
        self.queue_cycles
    }

    /// Sets the amount of queue cycles
    pub fn set_queue_cycles(&mut self, val: u64) {
        self.queue_cycles = val;
    }

    /// Gets the `n_fuzz`.
    #[must_use]
    pub fn n_fuzz(&self) -> &[u32] {
        &self.n_fuzz
    }

    /// Sets the `n_fuzz`.
    #[must_use]
    pub fn n_fuzz_mut(&mut self) -> &mut [u32] {
        &mut self.n_fuzz
    }
}

/// The power schedule to use
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum PowerSchedule {
    /// The `explore` power schedule
    EXPLORE,
    /// The `exploit` power schedule
    EXPLOIT,
    /// The `fast` power schedule
    FAST,
    /// The `coe` power schedule
    COE,
    /// The `lin` power schedule
    LIN,
    /// The `quad` power schedule
    QUAD,
}

/// A corpus scheduler using power schedules
/// Note that this corpus is merely holding the metadata necessary for the power calculation
/// and here we DON'T actually calculate the power (we do it in the stage)
#[derive(Clone, Debug)]
pub struct PowerQueueScheduler<C, O, S> {
    queue_cycles: u64,
    strat: PowerSchedule,
    map_observer_handle: Handle<C>,
    last_hash: usize,
    phantom: PhantomData<(O, S)>,
}

impl<C, O, S> UsesState for PowerQueueScheduler<C, O, S>
where
    S: State,
{
    type State = S;
}

impl<C, O, S> RemovableScheduler for PowerQueueScheduler<C, O, S>
where
    S: State + HasTestcase + HasMetadata + HasCorpus,
    O: MapObserver,
    C: AsRef<O>,
{
    /// This will *NOT* neutralize the effect of this removed testcase from the global data such as `SchedulerMetadata`
    fn on_remove(
        &mut self,
        _state: &mut Self::State,
        _id: CorpusId,
        _prev: &Option<Testcase<<Self::State as UsesInput>::Input>>,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// This will *NOT* neutralize the effect of this removed testcase from the global data such as `SchedulerMetadata`
    fn on_replace(
        &mut self,
        _state: &mut Self::State,
        _id: CorpusId,
        _prev: &Testcase<<Self::State as UsesInput>::Input>,
    ) -> Result<(), Error> {
        Ok(())
    }
}

impl<C, O, S> AflScheduler<C, O, S> for PowerQueueScheduler<C, O, S>
where
    S: HasCorpus + HasMetadata + HasTestcase + State,
    O: MapObserver,
    C: AsRef<O>,
{
    fn last_hash(&self) -> usize {
        self.last_hash
    }

    fn set_last_hash(&mut self, hash: usize) {
        self.last_hash = hash;
    }

    fn map_observer_handle(&self) -> &Handle<C> {
        &self.map_observer_handle
    }
}

impl<C, O, S> HasQueueCycles for PowerQueueScheduler<C, O, S>
where
    S: HasCorpus + HasMetadata + HasTestcase + State,
    O: MapObserver,
    C: AsRef<O>,
{
    fn queue_cycles(&self) -> u64 {
        self.queue_cycles
    }
}

impl<C, O, S> Scheduler for PowerQueueScheduler<C, O, S>
where
    S: HasCorpus + HasMetadata + HasTestcase + State,
    O: MapObserver,
    C: AsRef<O>,
{
    /// Called when a [`Testcase`] is added to the corpus
    fn on_add(&mut self, state: &mut Self::State, id: CorpusId) -> Result<(), Error> {
        self.on_add_metadata(state, id)
    }

    fn on_evaluation<OT>(
        &mut self,
        state: &mut Self::State,
        input: &<Self::State as UsesInput>::Input,
        observers: &OT,
    ) -> Result<(), Error>
    where
        OT: ObserversTuple<Self::State>,
    {
        self.on_evaluation_metadata(state, input, observers)
    }

    fn next(&mut self, state: &mut Self::State) -> Result<CorpusId, Error> {
        if state.corpus().count() == 0 {
            Err(Error::empty(
                "No entries in corpus. This often implies the target is not properly instrumented.",
            ))
        } else {
            let id = match state.corpus().current() {
                Some(cur) => {
                    if let Some(next) = state.corpus().next(*cur) {
                        next
                    } else {
                        self.queue_cycles += 1;
                        let psmeta = state.metadata_mut::<SchedulerMetadata>()?;
                        psmeta.set_queue_cycles(self.queue_cycles());
                        state.corpus().first().unwrap()
                    }
                }
                None => state.corpus().first().unwrap(),
            };
            self.set_current_scheduled(state, Some(id))?;

            Ok(id)
        }
    }

    /// Set current fuzzed corpus id and `scheduled_count`
    fn set_current_scheduled(
        &mut self,
        state: &mut Self::State,
        next_id: Option<CorpusId>,
    ) -> Result<(), Error> {
        self.on_next_metadata(state, next_id)?;

        *state.corpus_mut().current_mut() = next_id;
        Ok(())
    }
}

impl<C, O, S> PowerQueueScheduler<C, O, S>
where
    S: HasMetadata,
    O: MapObserver,
    C: AsRef<O> + Named,
{
    /// Create a new [`PowerQueueScheduler`]
    #[must_use]
    pub fn new(state: &mut S, map_observer: &C, strat: PowerSchedule) -> Self {
        if !state.has_metadata::<SchedulerMetadata>() {
            state.add_metadata::<SchedulerMetadata>(SchedulerMetadata::new(Some(strat)));
        }
        PowerQueueScheduler {
            queue_cycles: 0,
            strat,
            map_observer_handle: map_observer.handle(),
            last_hash: 0,
            phantom: PhantomData,
        }
    }

    /// Getter for `strat`
    #[must_use]
    pub fn strat(&self) -> &PowerSchedule {
        &self.strat
    }
}
