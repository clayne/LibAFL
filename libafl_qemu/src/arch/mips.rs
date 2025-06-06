use std::sync::OnceLock;

use enum_map::{EnumMap, enum_map};
use num_enum::{IntoPrimitive, TryFromPrimitive};
#[cfg(feature = "python")]
use pyo3::prelude::*;
pub use strum_macros::EnumIter;
pub use syscall_numbers::mips::*;

use crate::{CallingConvention, QemuRWError, QemuRWErrorKind, sync_exit::ExitArgs};

#[expect(non_upper_case_globals)]
impl CallingConvention {
    pub const Default: CallingConvention = CallingConvention::MipsO32;
}

/// Registers for the MIPS instruction set.
#[derive(IntoPrimitive, TryFromPrimitive, Debug, Copy, Clone, EnumIter)]
#[repr(i32)]
pub enum Regs {
    R0 = 0,
    At = 1,
    V0 = 2,
    V1 = 3,
    A0 = 4,
    A1 = 5,
    A2 = 6,
    A3 = 7,
    T0 = 8,
    T1 = 9,
    T2 = 10,
    T3 = 11,
    T4 = 12,
    T5 = 13,
    T6 = 14,
    T7 = 15,
    S0 = 16,
    S1 = 17,
    S2 = 18,
    S3 = 19,
    S4 = 20,
    S5 = 21,
    S6 = 22,
    S7 = 23,
    T8 = 24,
    T9 = 25,
    K0 = 26,
    K1 = 27,
    Gp = 28,
    Sp = 29,
    Fp = 30,
    Ra = 31,

    Pc = 37,
}

static EXIT_ARCH_REGS: OnceLock<EnumMap<ExitArgs, Regs>> = OnceLock::new();

pub fn get_exit_arch_regs() -> &'static EnumMap<ExitArgs, Regs> {
    EXIT_ARCH_REGS.get_or_init(|| {
        enum_map! {
            ExitArgs::Ret  => Regs::V0,
            ExitArgs::Cmd  => Regs::V0,
            ExitArgs::Arg1 => Regs::A0,
            ExitArgs::Arg2 => Regs::A1,
            ExitArgs::Arg3 => Regs::A2,
            ExitArgs::Arg4 => Regs::A3,
            ExitArgs::Arg5 => Regs::T0,
            ExitArgs::Arg6 => Regs::T1,
        }
    })
}

/// alias registers
#[expect(non_upper_case_globals)]
impl Regs {
    pub const Zero: Regs = Regs::R0;
}

/// Return an MIPS ArchCapstoneBuilder
pub fn capstone() -> capstone::arch::mips::ArchCapstoneBuilder {
    capstone::Capstone::new().mips()
}

pub type GuestReg = u32;

impl crate::ArchExtras for crate::CPU {
    fn read_return_address(&self) -> Result<GuestReg, QemuRWError> {
        self.read_reg(Regs::Ra)
    }

    fn write_return_address<T>(&self, val: T) -> Result<(), QemuRWError>
    where
        T: Into<GuestReg>,
    {
        self.write_reg(Regs::Ra, val)
    }

    fn read_function_argument_with_cc(
        &self,
        idx: u8,
        conv: CallingConvention,
    ) -> Result<GuestReg, QemuRWError> {
        QemuRWError::check_conv(QemuRWErrorKind::Read, CallingConvention::MipsO32, conv)?;

        let reg_id = match idx {
            0 => Regs::A0,
            1 => Regs::A1,
            2 => Regs::A2,
            3 => Regs::A3,
            // 4.. would be on the stack, let's not do this for now
            r => return Err(QemuRWError::new_argument_error(QemuRWErrorKind::Read, r)),
        };

        self.read_reg(reg_id)
    }

    fn write_function_argument_with_cc<T>(
        &self,
        idx: u8,
        val: T,
        conv: CallingConvention,
    ) -> Result<(), QemuRWError>
    where
        T: Into<GuestReg>,
    {
        QemuRWError::check_conv(QemuRWErrorKind::Write, CallingConvention::MipsO32, conv)?;

        let val: GuestReg = val.into();
        match idx {
            0 => self.write_reg(Regs::A0, val),
            1 => self.write_reg(Regs::A1, val),
            2 => self.write_reg(Regs::A2, val),
            3 => self.write_reg(Regs::A3, val),
            // 4.. would be on the stack, let's not do this for now
            r => Err(QemuRWError::new_argument_error(QemuRWErrorKind::Write, r)),
        }
    }
}
