#[cfg(unix)]
mod imp {
    use anyhow::{Context, Result};
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};

    pub(in crate::workspace) struct ProcessTree {
        process_group: Option<i32>,
    }

    impl ProcessTree {
        pub(in crate::workspace) fn prepare(command: &mut Command) -> Result<Self> {
            command.process_group(0);
            Ok(Self {
                process_group: None,
            })
        }

        pub(in crate::workspace) fn attach(&mut self, child: &Child) -> Result<()> {
            let process_group = child.id();
            self.process_group = Some(
                process_group
                    .try_into()
                    .context("Command process ID is too large for a Unix process group.")?,
            );
            Ok(())
        }

        pub(in crate::workspace) fn resource_usage(
            &self,
        ) -> Option<super::super::command::ResourceUsage> {
            None
        }

        pub(in crate::workspace) fn terminate(&mut self) -> Result<()> {
            let Some(process_group) = self.process_group else {
                return Ok(());
            };

            // A negative PID addresses every process in this process group. The
            // command was made the group leader by `process_group(0)` above.
            let result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
            if result == 0 {
                return Ok(());
            }

            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(())
            } else {
                Err(error).context("Failed to terminate Unix command process group.")
            }
        }
    }

    impl Drop for ProcessTree {
        fn drop(&mut self) {
            let _ = self.terminate();
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::super::command::ResourceUsage;
    use anyhow::{Context, Result};
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command};
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
        QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
    };

    pub(in crate::workspace) struct ProcessTree {
        job: OwnedHandle,
    }

    impl ProcessTree {
        pub(in crate::workspace) fn prepare(command: &mut Command) -> Result<Self> {
            // The process must not execute until it belongs to the Job Object;
            // otherwise it could create descendants which escape the job.
            command.creation_flags(CREATE_SUSPENDED);

            let raw_job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if raw_job.is_null() {
                return Err(std::io::Error::last_os_error())
                    .context("Failed to create Job Object.");
            }

            let job = unsafe { OwnedHandle::from_raw_handle(raw_job) };
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = unsafe {
                SetInformationJobObject(
                    job.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast::<c_void>(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if configured == 0 {
                return Err(std::io::Error::last_os_error())
                    .context("Failed to configure command Job Object.");
            }

            Ok(Self { job })
        }

        pub(in crate::workspace) fn attach(&mut self, child: &Child) -> Result<()> {
            let assigned = unsafe {
                AssignProcessToJobObject(self.job.as_raw_handle(), child.as_raw_handle())
            };
            if assigned == 0 {
                return Err(std::io::Error::last_os_error())
                    .context("Failed to assign command to Job Object.");
            }

            let process_id = child.id();
            resume_process_threads(process_id)
                .context("Failed to resume command after assigning its Job Object.")
        }

        pub(in crate::workspace) fn resource_usage(&self) -> Option<ResourceUsage> {
            let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            let accounting_ok = unsafe {
                QueryInformationJobObject(
                    self.job.as_raw_handle(),
                    JobObjectBasicAccountingInformation,
                    (&raw mut accounting).cast::<c_void>(),
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    std::ptr::null_mut(),
                )
            } != 0;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            let limits_ok = unsafe {
                QueryInformationJobObject(
                    self.job.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    (&raw mut limits).cast::<c_void>(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                    std::ptr::null_mut(),
                )
            } != 0;
            if !accounting_ok && !limits_ok {
                return None;
            }
            let ticks_to_duration = |ticks: i64| {
                u64::try_from(ticks)
                    .ok()
                    .and_then(|ticks| ticks.checked_mul(100))
                    .map(Duration::from_nanos)
            };
            Some(ResourceUsage {
                user: accounting_ok
                    .then(|| ticks_to_duration(accounting.TotalUserTime))
                    .flatten(),
                system: accounting_ok
                    .then(|| ticks_to_duration(accounting.TotalKernelTime))
                    .flatten(),
                peak_memory_bytes: limits_ok.then_some(limits.PeakJobMemoryUsed as u64),
            })
        }

        pub(in crate::workspace) fn terminate(&mut self) -> Result<()> {
            let terminated = unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) };
            if terminated == 0 {
                Err(std::io::Error::last_os_error())
                    .context("Failed to terminate command Job Object.")
            } else {
                Ok(())
            }
        }
    }

    fn resume_process_threads(process_id: u32) -> Result<()> {
        let raw_snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if raw_snapshot == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error())
                .context("Failed to enumerate command threads.");
        }
        let snapshot = unsafe { OwnedHandle::from_raw_handle(raw_snapshot) };
        let mut entry = THREADENTRY32 {
            dwSize: size_of::<THREADENTRY32>() as u32,
            ..THREADENTRY32::default()
        };

        if unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } == 0 {
            return Err(std::io::Error::last_os_error())
                .context("Failed to read the first command thread.");
        }

        let mut resumed = false;
        loop {
            if entry.th32OwnerProcessID == process_id {
                let raw_thread =
                    unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if raw_thread.is_null() {
                    return Err(std::io::Error::last_os_error())
                        .context("Failed to open a suspended command thread.");
                }
                let thread = unsafe { OwnedHandle::from_raw_handle(raw_thread) };
                if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                    return Err(std::io::Error::last_os_error())
                        .context("Failed to resume a command thread.");
                }
                resumed = true;
            }

            if unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } != 0 {
                continue;
            }

            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                break;
            }
            return Err(error).context("Failed while enumerating command threads.");
        }

        if resumed {
            Ok(())
        } else {
            anyhow::bail!("Suspended command thread was not found.")
        }
    }
}

pub(super) use imp::ProcessTree;
