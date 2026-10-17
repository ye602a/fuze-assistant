use crate::Result;
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    process::Child,
};
use windows_sys::Win32::System::JobObjects::*;
pub struct Job(OwnedHandle);
impl Job {
    pub fn new() -> Result<Self> {
        Self::with_flags(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)
    }
    /// Own only the UI session. Browser/file-manager launches must survive it;
    /// OCR children are separately assigned to their own strict worker job.
    pub fn for_ui_session() -> Result<Self> {
        Self::with_flags(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK)
    }
    fn with_flags(flags: u32) -> Result<Self> {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err("worker-job-create");
            }
            let job = Self(OwnedHandle::from_raw_handle(handle));
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = flags;
            if SetInformationJobObject(
                job.0.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            {
                return Err("worker-job-limits");
            }
            Ok(job)
        }
    }
    pub fn assign(&self, child: &Child) -> Result<()> {
        if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), child.as_raw_handle()) } == 0 {
            Err("worker-job-assign")
        } else {
            Ok(())
        }
    }
}
