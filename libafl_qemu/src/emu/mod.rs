//! Higher-level abstraction of [`Qemu`]
//!
//! [`Emulator`] is built above [`Qemu`] and provides convenient abstractions.

use core::fmt::{self, Debug, Display, Formatter};
use std::{cell::RefCell, ops::Add, pin::Pin};

use hashbrown::HashMap;
use libafl::{
    executors::ExitKind, inputs::HasTargetBytes, observers::ObserversTuple, state::HasExecutions,
};
use libafl_qemu_sys::{GuestAddr, GuestPhysAddr, GuestUsize, GuestVirtAddr};

#[cfg(doc)]
use crate::modules::EmulatorModule;
use crate::{
    CPU, Qemu, QemuExitError, QemuExitReason, QemuHooks, QemuInitError, QemuMemoryChunk,
    QemuParams, QemuShutdownCause, Regs,
    breakpoint::{Breakpoint, BreakpointId},
    command::{CommandError, CommandManager, NopCommandManager, StdCommandManager},
    modules::EmulatorModuleTuple,
    sync_exit::CustomInsn,
};

mod hooks;
pub use hooks::*;

mod builder;
pub use builder::*;

mod drivers;
pub use drivers::*;

mod snapshot;
pub use snapshot::*;

#[cfg(feature = "usermode")]
mod usermode;
#[cfg(feature = "usermode")]
pub use usermode::*;

#[cfg(feature = "systemmode")]
mod systemmode;
#[cfg(feature = "systemmode")]
pub use systemmode::*;

use crate::config::QemuConfigBuilder;

#[derive(Copy, Clone)]
pub enum GuestAddrKind {
    Physical(GuestPhysAddr),
    Virtual(GuestVirtAddr),
}

#[derive(Clone)]
pub enum EmulatorExitResult<C> {
    QemuExit(QemuShutdownCause), // QEMU ended for some reason.
    Breakpoint(Breakpoint<C>),   // Breakpoint triggered. Contains the address of the trigger.
    CustomInsn(CustomInsn<C>), // Synchronous backdoor: The guest triggered a backdoor and should return to LibAFL.
    Crash,                     // Crash
    Timeout,                   // Timeout
}

impl<C> Debug for EmulatorExitResult<C>
where
    C: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            EmulatorExitResult::QemuExit(qemu_exit) => {
                write!(f, "{qemu_exit:?}")
            }
            EmulatorExitResult::Breakpoint(bp) => {
                write!(f, "{bp:?}")
            }
            EmulatorExitResult::CustomInsn(sync_exit) => {
                write!(f, "{sync_exit:?}")
            }
            EmulatorExitResult::Crash => {
                write!(f, "Crash")
            }
            EmulatorExitResult::Timeout => {
                write!(f, "Timeout")
            }
        }
    }
}
#[derive(Debug, Clone)]
pub enum EmulatorExitError {
    UnknownKind,
    UnexpectedExit,
    CommandError(CommandError),
    BreakpointNotFound(GuestAddr),
}

#[derive(Debug, Clone)]
pub struct InputLocation {
    mem_chunk: QemuMemoryChunk,
    cpu: CPU,
    ret_register: Option<Regs>,
}

/// The high-level interface to [`Qemu`].
///
/// It embeds multiple structures aiming at making QEMU usage easier:
///
/// - An [`IsSnapshotManager`] implementation, implementing the QEMU snapshot method to use.
/// - An [`EmulatorDriver`] implementation, responsible for handling the high-level control flow of QEMU runtime.
/// - A [`CommandManager`] implementation, handling the commands received from the target.
/// - [`EmulatorModules`], containing the [`EmulatorModule`] implementations' state.
///
/// Each of these fields can be set manually to finely tune how QEMU is getting handled.
/// It is highly encouraged to build [`Emulator`] using the associated [`EmulatorBuilder`].
/// There are two main functions to access the builder:
///
/// - [`Emulator::builder`] gives access to the standard [`EmulatorBuilder`], embedding all the standard components of an [`Emulator`].
/// - [`Emulator::empty`] gives access to an empty [`EmulatorBuilder`]. This is mostly useful to create a more custom [`Emulator`].
///
/// Please check the documentation of [`EmulatorBuilder`] for more details.
#[derive(Debug)]
pub struct Emulator<C, CM, ED, ET, I, S, SM> {
    snapshot_manager: SM,
    modules: Pin<Box<EmulatorModules<ET, I, S>>>,
    command_manager: CM,
    driver: ED,
    breakpoints_by_addr: RefCell<HashMap<GuestAddr, Breakpoint<C>>>, // TODO: change to RC here
    breakpoints_by_id: RefCell<HashMap<BreakpointId, Breakpoint<C>>>,
    qemu: Qemu,
}

impl<C> EmulatorDriverResult<C> {
    #[must_use]
    pub fn end_of_run(&self) -> Option<ExitKind> {
        match self {
            EmulatorDriverResult::EndOfRun(exit_kind) => Some(*exit_kind),
            _ => None,
        }
    }
}

impl Debug for GuestAddrKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            GuestAddrKind::Physical(paddr) => write!(f, "paddr {paddr:#x}"),
            GuestAddrKind::Virtual(vaddr) => write!(f, "vaddr {vaddr:#x}"),
        }
    }
}

impl Add<GuestUsize> for GuestAddrKind {
    type Output = Self;

    fn add(self, rhs: GuestUsize) -> Self::Output {
        match self {
            GuestAddrKind::Physical(paddr) => GuestAddrKind::Physical(paddr + rhs as GuestPhysAddr),
            GuestAddrKind::Virtual(vaddr) => GuestAddrKind::Virtual(vaddr + rhs as GuestVirtAddr),
        }
    }
}

impl Display for GuestAddrKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            GuestAddrKind::Physical(phys_addr) => write!(f, "hwaddr 0x{phys_addr:x}"),
            GuestAddrKind::Virtual(virt_addr) => write!(f, "vaddr 0x{virt_addr:x}"),
        }
    }
}

impl From<SnapshotManagerError> for EmulatorDriverError {
    fn from(sm_error: SnapshotManagerError) -> Self {
        EmulatorDriverError::SMError(sm_error)
    }
}

impl From<SnapshotManagerCheckError> for EmulatorDriverError {
    fn from(sm_check_error: SnapshotManagerCheckError) -> Self {
        EmulatorDriverError::SMCheckError(sm_check_error)
    }
}

impl InputLocation {
    #[must_use]
    pub fn new(mem_chunk: QemuMemoryChunk, cpu: CPU, ret_register: Option<Regs>) -> Self {
        Self {
            mem_chunk,
            cpu,
            ret_register,
        }
    }

    #[must_use]
    pub fn mem_chunk(&self) -> &QemuMemoryChunk {
        &self.mem_chunk
    }

    #[must_use]
    pub fn ret_register(&self) -> &Option<Regs> {
        &self.ret_register
    }
}

impl From<EmulatorExitError> for EmulatorDriverError {
    fn from(error: EmulatorExitError) -> Self {
        EmulatorDriverError::QemuExitReasonError(error)
    }
}

impl From<CommandError> for EmulatorDriverError {
    fn from(error: CommandError) -> Self {
        EmulatorDriverError::CommandError(error)
    }
}

impl<C> Display for EmulatorExitResult<C>
where
    C: Debug,
{
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            EmulatorExitResult::QemuExit(shutdown_cause) => write!(f, "End: {shutdown_cause:?}"),
            EmulatorExitResult::Breakpoint(bp) => write!(f, "{bp}"),
            EmulatorExitResult::CustomInsn(sync_exit) => {
                write!(f, "Sync exit: {sync_exit:?}")
            }
            EmulatorExitResult::Crash => {
                write!(f, "Crash")
            }
            EmulatorExitResult::Timeout => {
                write!(f, "Timeout")
            }
        }
    }
}

impl From<CommandError> for EmulatorExitError {
    fn from(error: CommandError) -> Self {
        EmulatorExitError::CommandError(error)
    }
}

impl<C, I, S> Emulator<C, NopCommandManager, NopEmulatorDriver, (), I, S, NopSnapshotManager> {
    #[must_use]
    pub fn empty() -> EmulatorBuilder<
        C,
        NopCommandManager,
        NopEmulatorDriver,
        (),
        QemuConfigBuilder,
        I,
        S,
        NopSnapshotManager,
    > {
        EmulatorBuilder::empty()
    }
}

impl<C, I, S> Emulator<C, StdCommandManager<S>, StdEmulatorDriver, (), I, S, StdSnapshotManager>
where
    S: HasExecutions + Unpin,
    I: HasTargetBytes,
{
    #[must_use]
    pub fn builder() -> EmulatorBuilder<
        C,
        StdCommandManager<S>,
        StdEmulatorDriver,
        (),
        QemuConfigBuilder,
        I,
        S,
        StdSnapshotManager,
    > {
        EmulatorBuilder::default()
    }
}

impl<C, CM, ED, ET, I, S, SM> Emulator<C, CM, ED, ET, I, S, SM> {
    pub fn modules(&self) -> &EmulatorModules<ET, I, S> {
        &self.modules
    }

    #[must_use]
    pub fn qemu(&self) -> Qemu {
        self.qemu
    }

    #[must_use]
    pub fn driver(&self) -> &ED {
        &self.driver
    }

    #[must_use]
    pub fn driver_mut(&mut self) -> &mut ED {
        &mut self.driver
    }

    #[must_use]
    pub fn snapshot_manager(&self) -> &SM {
        &self.snapshot_manager
    }

    #[must_use]
    pub fn snapshot_manager_mut(&mut self) -> &mut SM {
        &mut self.snapshot_manager
    }

    pub fn command_manager(&self) -> &CM {
        &self.command_manager
    }

    pub fn command_manager_mut(&mut self) -> &mut CM {
        &mut self.command_manager
    }
}

impl<C, CM, ED, ET, I, S, SM> Emulator<C, CM, ED, ET, I, S, SM>
where
    ET: Unpin,
    I: Unpin,
    S: Unpin,
{
    pub fn modules_mut(&mut self) -> &mut EmulatorModules<ET, I, S> {
        self.modules.as_mut().get_mut()
    }
}

impl<C, CM, ED, ET, I, S, SM> Emulator<C, CM, ED, ET, I, S, SM>
where
    ET: EmulatorModuleTuple<I, S>,
    I: Unpin,
    S: Unpin,
{
    #[allow(clippy::must_use_candidate, clippy::similar_names)]
    pub fn new<T>(
        qemu_params: T,
        modules: ET,
        driver: ED,
        snapshot_manager: SM,
        command_manager: CM,
    ) -> Result<Self, QemuInitError>
    where
        T: Into<QemuParams>,
    {
        let mut qemu_params = qemu_params.into();

        // # Safety
        // `QemuHooks` can be used without QEMU being fully initialized, we make sure to only call
        // functions that do not depend on whether QEMU is well-initialized or not.
        let emulator_hooks = unsafe { EmulatorHooks::new(QemuHooks::get_unchecked()) };

        // # Safety
        // This is the only call to `EmulatorModules::new`.
        // Since Emulator can only be created once, we fulfil the conditions to call this function.
        let mut emulator_modules = unsafe { EmulatorModules::new(emulator_hooks, modules) };

        // # Safety
        // This is mostly safe, but can cause issues if module hooks call to emulator_modules.modules_mut().
        // In that case, it would cause the creation of a double mutable reference.
        // We need to refactor Modules to avoid such problem in the future at some point.
        // TODO: fix things there properly. The biggest issue being that it creates 2 mut ref to the module with the callback being called
        unsafe {
            emulator_modules.modules_mut().pre_qemu_init_all(
                EmulatorModules::<ET, I, S>::emulator_modules_mut_unchecked(),
                &mut qemu_params,
            );
        }

        let qemu = Qemu::init(qemu_params)?;

        // # Safety
        // Pre-init hooks have been called above.
        unsafe {
            Ok(Self::new_with_qemu(
                qemu,
                emulator_modules,
                driver,
                snapshot_manager,
                command_manager,
            ))
        }
    }

    /// New emulator with already initialized QEMU.
    /// We suppose modules init hooks have already been run.
    ///
    /// # Safety
    ///
    /// pre-init qemu hooks should be run before calling this.
    unsafe fn new_with_qemu(
        qemu: Qemu,
        emulator_modules: Pin<Box<EmulatorModules<ET, I, S>>>,
        driver: ED,
        snapshot_manager: SM,
        command_manager: CM,
    ) -> Self {
        let mut emulator = Emulator {
            modules: emulator_modules,
            command_manager,
            snapshot_manager,
            driver,
            breakpoints_by_addr: RefCell::new(HashMap::new()),
            breakpoints_by_id: RefCell::new(HashMap::new()),
            qemu,
        };

        emulator.modules.post_qemu_init_all(qemu);

        emulator
    }
}

impl<C, CM, ED, ET, I, S, SM> Emulator<C, CM, ED, ET, I, S, SM>
where
    C: Clone,
    CM: CommandManager<C, ED, ET, I, S, SM, Commands = C>,
    ED: EmulatorDriver<C, CM, ET, I, S, SM>,
    ET: EmulatorModuleTuple<I, S>,
    I: Unpin,
    S: Unpin,
{
    /// This function will run the emulator until the exit handler decides to stop the execution for
    /// whatever reason, depending on the choosen handler.
    /// It is a higher-level abstraction of [`Emulator::run`] that will take care of some part of the runtime logic,
    /// returning only when something interesting happen.
    ///
    /// # Safety
    /// Should, in general, be safe to call.
    /// Of course, the emulated target is not contained securely and can corrupt state or interact with the operating system.
    pub unsafe fn run(
        &mut self,
        state: &mut S,
        input: &I,
    ) -> Result<EmulatorDriverResult<C>, EmulatorDriverError> {
        loop {
            // Insert input if the location is already known
            ED::pre_qemu_exec(self, input);

            // Run QEMU
            log::debug!("Running QEMU...");
            let mut exit_reason = unsafe { self.run_qemu() };
            log::debug!("QEMU stopped.");

            // Handle QEMU exit
            if let Some(exit_handler_result) =
                ED::post_qemu_exec(self, state, &mut exit_reason, input)?
            {
                return Ok(exit_handler_result);
            }
        }
    }

    /// This function will run the emulator until the next breakpoint, or until finish.
    ///
    /// # Safety
    ///
    /// Should, in general, be safe to call.
    /// Of course, the emulated target is not contained securely and can corrupt state or interact with the operating system.
    pub unsafe fn run_qemu(&self) -> Result<EmulatorExitResult<C>, EmulatorExitError> {
        match unsafe { self.qemu.run() } {
            Ok(qemu_exit_reason) => Ok(match qemu_exit_reason {
                QemuExitReason::End(qemu_shutdown_cause) => {
                    EmulatorExitResult::QemuExit(qemu_shutdown_cause)
                }
                QemuExitReason::Crash => EmulatorExitResult::Crash,
                QemuExitReason::Timeout => EmulatorExitResult::Timeout,
                QemuExitReason::Breakpoint(bp_addr) => {
                    let bp = self
                        .breakpoints_by_addr
                        .borrow()
                        .get(&bp_addr)
                        .ok_or(EmulatorExitError::BreakpointNotFound(bp_addr))?
                        .clone();
                    EmulatorExitResult::Breakpoint(bp.clone())
                }
                QemuExitReason::SyncExit => EmulatorExitResult::CustomInsn(CustomInsn::new(
                    self.command_manager.parse(self.qemu)?,
                )),
            }),
            Err(qemu_exit_reason_error) => Err(match qemu_exit_reason_error {
                QemuExitError::UnexpectedExit => EmulatorExitError::UnexpectedExit,
                QemuExitError::UnknownKind => EmulatorExitError::UnknownKind,
            }),
        }
    }

    /// First exec of Emulator, called before calling to user harness the first time
    pub fn first_exec(&mut self, state: &mut S) {
        ED::first_harness_exec(self, state);
    }

    /// Pre exec of Emulator, called before calling to user harness
    pub fn pre_exec(&mut self, state: &mut S, input: &I) {
        ED::pre_harness_exec(self, state, input);
    }

    /// Post exec of Emulator, called before calling to user harness
    pub fn post_exec<OT>(
        &mut self,
        input: &I,
        observers: &mut OT,
        state: &mut S,
        exit_kind: &mut ExitKind,
    ) where
        OT: ObserversTuple<I, S>,
    {
        ED::post_harness_exec(self, input, observers, state, exit_kind);
    }
}

impl<C, CM, ED, ET, I, S, SM> Emulator<C, CM, ED, ET, I, S, SM> {
    pub fn add_breakpoint(&self, mut bp: Breakpoint<C>, enable: bool) -> BreakpointId
    where
        C: Clone,
    {
        if enable {
            bp.enable(self.qemu);
        }

        let bp_id = bp.id();
        let bp_addr = bp.addr();

        assert!(
            self.breakpoints_by_addr
                .borrow_mut()
                .insert(bp_addr, bp.clone())
                .is_none(),
            "Adding multiple breakpoints at the same address"
        );

        assert!(
            self.breakpoints_by_id
                .borrow_mut()
                .insert(bp_id, bp)
                .is_none(),
            "Adding the same breakpoint multiple times"
        );

        bp_id
    }

    pub fn remove_breakpoint(&self, bp_id: BreakpointId) {
        let bp_addr = {
            let mut bp_map = self.breakpoints_by_id.borrow_mut();
            let bp = bp_map.get_mut(&bp_id).expect("Did not find the breakpoint");
            bp.disable(self.qemu);
            bp.addr()
        };

        self.breakpoints_by_id
            .borrow_mut()
            .remove(&bp_id)
            .expect("Could not remove bp");
        self.breakpoints_by_addr
            .borrow_mut()
            .remove(&bp_addr)
            .expect("Could not remove bp");
    }
}
