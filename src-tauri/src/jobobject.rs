#[cfg(windows)]
pub fn init_job_object() {
    use std::mem;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            crate::applog::event("jobobject: CreateJobObjectW failed — proxy cleanup falls back to taskkill");
            return;
        }

        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ret = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const core::ffi::c_void,
            mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ret == 0 {
            crate::applog::event("jobobject: SetInformationJobObject failed");
            return;
        }

        if AssignProcessToJobObject(job, GetCurrentProcess()) == 0 {
            crate::applog::event("jobobject: AssignProcessToJobObject failed — proxy cleanup falls back to taskkill");
            return;
        }

        let _ = job;
        crate::applog::event("jobobject: kill-on-close job active — all child processes die with VelocityRL");
    }
}

#[cfg(not(windows))]
pub fn init_job_object() {}
