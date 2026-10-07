//! Command endpoints belong to one Windows logon, including separate RDP logons
//! of the same account. The pipe name is discovery; the DACL enforces access.
use std::{mem::size_of, os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle}};
use windows::Win32::{Foundation::{HANDLE, ERROR_INSUFFICIENT_BUFFER}, Security::*,
    Storage::FileSystem::FILE_ALL_ACCESS, System::Threading::{GetCurrentProcess, OpenProcessToken}};

pub(super) struct Logon(Vec<usize>);
impl Logon {
    pub fn current() -> Result<Self, String> {
        unsafe {
            let mut handle = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle).map_err(|e| e.to_string())?;
            let token = OwnedHandle::from_raw_handle(handle.0);
            let mut needed = 0;
            let result = GetTokenInformation(HANDLE(token.as_raw_handle()), TokenLogonSid, None, 0, &mut needed);
            if result.as_ref().err().is_none_or(|e| e.code() != ERROR_INSUFFICIENT_BUFFER.to_hresult())
                || needed < size_of::<TOKEN_GROUPS>() as u32 {
                return Err("could not size the current logon identity".into());
            }
            let mut groups = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
            GetTokenInformation(HANDLE(token.as_raw_handle()), TokenLogonSid,
                Some(groups.as_mut_ptr().cast()), needed, &mut needed).map_err(|e| e.to_string())?;
            let groups = &*(groups.as_ptr().cast::<TOKEN_GROUPS>());
            if groups.GroupCount != 1 || !IsValidSid(groups.Groups[0].Sid).as_bool() {
                return Err("the command endpoint requires one valid Windows logon identity".into());
            }
            let length = GetLengthSid(groups.Groups[0].Sid);
            let mut sid = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
            CopySid(length, PSID(sid.as_mut_ptr().cast()), groups.Groups[0].Sid).map_err(|e| e.to_string())?;
            Ok(Self(sid))
        }
    }
    fn sid(&self) -> PSID { PSID(self.0.as_ptr().cast_mut().cast()) }
    pub fn hash(&self, initial: u64) -> u64 {
        let bytes = unsafe { std::slice::from_raw_parts(self.sid().0.cast::<u8>(), GetLengthSid(self.sid()) as usize) };
        bytes.iter().fold(initial, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3))
    }
}

pub(super) struct Security {
    descriptor: SECURITY_DESCRIPTOR,
    // The descriptor borrows the stable heap allocation until CreateNamedPipeW returns.
    _acl: Vec<usize>,
}
impl Security {
    pub fn new() -> Result<Self, String> {
        let logon = Logon::current()?;
        let length = size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>()
            + unsafe { GetLengthSid(logon.sid()) } as usize;
        let mut storage = vec![0usize; length.div_ceil(size_of::<usize>())];
        let acl = storage.as_mut_ptr().cast::<ACL>();
        let mut descriptor = SECURITY_DESCRIPTOR::default();
        unsafe {
            InitializeAcl(acl, length as u32, ACL_REVISION).map_err(|e| e.to_string())?;
            AddAccessAllowedAce(acl, ACL_REVISION, FILE_ALL_ACCESS.0, logon.sid()).map_err(|e| e.to_string())?;
            InitializeSecurityDescriptor(PSECURITY_DESCRIPTOR((&mut descriptor as *mut SECURITY_DESCRIPTOR).cast()), 1)
                .map_err(|e| e.to_string())?;
            SetSecurityDescriptorDacl(PSECURITY_DESCRIPTOR((&mut descriptor as *mut SECURITY_DESCRIPTOR).cast()), true, Some(acl), false)
                .map_err(|e| e.to_string())?;
        }
        Ok(Self { descriptor, _acl: storage })
    }
    pub fn attributes(&mut self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: (&mut self.descriptor as *mut SECURITY_DESCRIPTOR).cast(), bInheritHandle: false.into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;

    #[test]
    fn native_pipe_grants_only_the_current_logon() {
        let name = format!("logon ACL {}", std::process::id());
        let path = super::super::pipe_path(&name).unwrap();
        let pipe = super::super::bind_path(&path).unwrap();
        let mut needed = 0;
        let handle = HANDLE(pipe.as_raw_handle());
        let error = unsafe { GetKernelObjectSecurity(handle, DACL_SECURITY_INFORMATION.0, None, 0, &mut needed) }.unwrap_err();
        assert_eq!(error.code(), ERROR_INSUFFICIENT_BUFFER.to_hresult());
        let mut buffer = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
        let descriptor = PSECURITY_DESCRIPTOR(buffer.as_mut_ptr().cast());
        unsafe { GetKernelObjectSecurity(handle, DACL_SECURITY_INFORMATION.0, Some(descriptor), needed, &mut needed) }.unwrap();
        let (mut present, mut defaulted, mut acl) = (false.into(), false.into(), std::ptr::null_mut());
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) }.unwrap();
        assert!(present.as_bool() && !acl.is_null());
        assert_eq!(unsafe { (*acl).AceCount }, 1, "no Everyone, anonymous or other logon grants");
        let mut entry = std::ptr::null_mut();
        unsafe { GetAce(acl, 0, &mut entry) }.unwrap();
        let entry = unsafe { &*(entry.cast::<ACCESS_ALLOWED_ACE>()) };
        assert_eq!(entry.Header.AceType, 0);
        assert_eq!(entry.Mask, FILE_ALL_ACCESS.0);
        let logon = Logon::current().unwrap();
        unsafe { EqualSid(PSID((&entry.SidStart as *const u32).cast_mut().cast()), logon.sid()) }.unwrap();
        assert_eq!(logon.hash(0), Logon::current().unwrap().hash(0));

        // A token whose logon group cannot grant access must fail even for a
        // read-only client. Impersonation never escapes this owned test thread.
        let rejected = std::thread::spawn(move || unsafe {
            let logon = Logon::current().unwrap();
            let mut source = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY | TOKEN_DUPLICATE, &mut source).unwrap();
            let source = OwnedHandle::from_raw_handle(source.0);
            let mut restricted = HANDLE::default();
            CreateRestrictedToken(HANDLE(source.as_raw_handle()), DISABLE_MAX_PRIVILEGE,
                Some(&[SID_AND_ATTRIBUTES { Sid: logon.sid(), Attributes: 0 }]), None, None, &mut restricted).unwrap();
            let restricted = OwnedHandle::from_raw_handle(restricted.0);
            ImpersonateLoggedOnUser(HANDLE(restricted.as_raw_handle())).unwrap();
            struct Revert;
            impl Drop for Revert { fn drop(&mut self) { unsafe { RevertToSelf() }.expect("test impersonation must end"); } }
            let _revert = Revert;
            OpenOptions::new().read(true).open(&path).unwrap_err().raw_os_error()
        }).join().unwrap();
        assert_eq!(rejected, Some(5), "Windows must deny a client without the permitted logon group");
        assert!(!super::super::accept_ready(&pipe));
        let path = super::super::pipe_path(&name).unwrap();
        let client = OpenOptions::new().read(true).write(true).open(path).unwrap();
        assert!(super::super::accept_ready(&pipe), "the normal caller still connects after the rejection");
        drop(client);
    }
}
