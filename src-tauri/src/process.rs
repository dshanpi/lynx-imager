use std::process::{Child, Command};

/// Configure a child process that runs behind the GUI without opening a
/// separate console window. Other platforms do not need special handling.
pub(crate) fn configure_background_command(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        // Isolate helpers in their own process group so cancellation also
        // terminates any subprocess spawned by a vendor CLI.
        command.process_group(0);
    }

    #[cfg(not(any(windows, unix)))]
    let _ = command;
}

/// Own a Windows Job Object for a hardware helper process. Closing or
/// explicitly terminating the job kills every process in the tree, including
/// a helper blocked inside a vendor USB driver. Unix helpers run in a private
/// process group so their subprocesses are terminated together; other
/// platforms keep the same API and rely on the direct child lifecycle.
pub(crate) struct ProcessJob {
    #[cfg(windows)]
    handle: windows::Win32::Foundation::HANDLE,
    #[cfg(windows)]
    process_id: u32,
    #[cfg(unix)]
    process_group: libc::pid_t,
}

impl ProcessJob {
    pub(crate) fn attach(child: &Child) -> Result<Self, String> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows::{
                core::PCWSTR,
                Win32::{
                    Foundation::HANDLE,
                    System::JobObjects::{
                        AssignProcessToJobObject, CreateJobObjectW,
                        JobObjectExtendedLimitInformation, SetInformationJobObject,
                        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    },
                },
            };

            let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
                .map_err(|error| format!("PROCESS_JOB_CREATE:{error}"))?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if let Err(error) = unsafe {
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(limits).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )
            } {
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(handle);
                }
                return Err(format!("PROCESS_JOB_CONFIGURE:{error}"));
            }
            let process = HANDLE(child.as_raw_handle());
            if let Err(error) = unsafe { AssignProcessToJobObject(handle, process) } {
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(handle);
                }
                return Err(format!("PROCESS_JOB_ASSIGN:{error}"));
            }
            return Ok(Self {
                handle,
                process_id: child.id(),
            });
        }

        #[cfg(not(windows))]
        {
            #[cfg(unix)]
            {
                let process_group = libc::pid_t::try_from(child.id())
                    .map_err(|_| "PROCESS_GROUP_ID_INVALID".to_owned())?;
                Ok(Self { process_group })
            }

            #[cfg(not(unix))]
            {
                let _ = child;
                Ok(Self {})
            }
        }
    }

    /// Ask Windows to cancel synchronous driver I/O issued by every thread in
    /// the helper before terminating its Job Object. Some legacy USB drivers
    /// leave `CreateFile` pending; terminating the process alone can then leave
    /// an unkillable-looking helper until the USB device or Windows restarts.
    pub(crate) fn cancel_blocking_io(&self) -> Result<(), String> {
        #[cfg(windows)]
        {
            use windows::Win32::{
                Foundation::CloseHandle,
                System::{
                    Diagnostics::ToolHelp::{
                        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                        THREADENTRY32,
                    },
                    Threading::{OpenThread, THREAD_TERMINATE},
                    IO::CancelSynchronousIo,
                },
            };

            let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }
                .map_err(|error| format!("PROCESS_THREAD_SNAPSHOT:{error}"))?;
            let mut entry = THREADENTRY32 {
                dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            let mut current = unsafe { Thread32First(snapshot, &mut entry) };
            while current.is_ok() {
                if entry.th32OwnerProcessID == self.process_id {
                    if let Ok(thread) =
                        unsafe { OpenThread(THREAD_TERMINATE, false, entry.th32ThreadID) }
                    {
                        // ERROR_NOT_FOUND means this thread has no cancellable
                        // synchronous request, which is a valid outcome.
                        let _ = unsafe { CancelSynchronousIo(thread) };
                        unsafe {
                            let _ = CloseHandle(thread);
                        }
                    }
                }
                current = unsafe { Thread32Next(snapshot, &mut entry) };
            }
            unsafe {
                let _ = CloseHandle(snapshot);
            }
        }
        Ok(())
    }

    pub(crate) fn terminate(&self) -> Result<(), String> {
        #[cfg(windows)]
        unsafe {
            windows::Win32::System::JobObjects::TerminateJobObject(self.handle, 1)
                .map_err(|error| format!("PROCESS_JOB_TERMINATE:{error}"))?;
        }
        #[cfg(unix)]
        {
            let result = unsafe { libc::kill(-self.process_group, libc::SIGKILL) };
            if result != 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(format!("PROCESS_GROUP_TERMINATE:{error}"));
                }
            }
        }
        Ok(())
    }
}

impl Drop for ProcessJob {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}
