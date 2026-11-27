//! Who signed an executable. Windows verifies the Authenticode signature; only the signer's
//! organisation name is then compared, exactly, with the one expected.
use std::path::Path;

/// The organisation named by the file's Authenticode signer, or `None` when Windows does
/// not verify the signature.
#[cfg(windows)]
#[allow(dead_code)] // used to check Wallpaper Engine's signature
pub(crate) fn signer_organization(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Security::Cryptography::{CertGetNameStringW, CERT_NAME_ATTR_TYPE};
    use windows::Win32::Security::WinTrust::{
        WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2,
        WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
        WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    };

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(wide.as_ptr()),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let verify = |data: &mut WINTRUST_DATA, action: &mut windows::core::GUID| unsafe {
        WinVerifyTrust(HWND(std::ptr::null_mut()), action, data as *mut WINTRUST_DATA as *mut _)
    };
    let mut organisation = None;
    if verify(&mut data, &mut action) == 0 {
        unsafe {
            let provider = WTHelperProvDataFromStateData(data.hWVTStateData);
            let signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0);
            if !signer.is_null() && !(*signer).pasCertChain.is_null() {
                let cert = (*(*signer).pasCertChain).pCert;
                // szOID_ORGANIZATION_NAME
                let oid = b"2.5.4.10\0";
                let mut buf = [0u16; 256];
                let len = CertGetNameStringW(cert, CERT_NAME_ATTR_TYPE, 0, Some(oid.as_ptr() as *const _), Some(&mut buf));
                if len > 1 {
                    organisation = Some(String::from_utf16_lossy(&buf[..len as usize - 1]));
                }
            }
        }
    }
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    verify(&mut data, &mut action);
    organisation
}

#[cfg(not(windows))]
pub(crate) fn signer_organization(_path: &Path) -> Option<String> {
    None
}

/// Whether `organization` is exactly the expected publisher: no case folding, no trimming,
/// no prefix or suffix, so a lookalike name never passes.
#[allow(dead_code)]
pub(crate) fn is_publisher(organization: &str, expected: &str) -> bool {
    !expected.is_empty() && organization == expected
}

/// True when Windows verifies the file's signature and its signer is exactly `expected`.
#[allow(dead_code)]
pub(crate) fn signed_by(path: &Path, expected: &str) -> bool {
    signer_organization(path).is_some_and(|organization| is_publisher(&organization, expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_exact_publisher_name() {
        assert!(is_publisher("Skutta Software GmbH", "Skutta Software GmbH"));
        for lookalike in [
            "",
            "skutta software gmbh",
            "Skutta Software GmbH ",
            " Skutta Software GmbH",
            "Skutta Software",
            "Skutta Software GmbH.",
            "Skutta Software GmbH (fake)",
            "Not Skutta Software GmbH",
            "Skutta  Software GmbH",
            "Skutta Softw\u{0430}re GmbH",
        ] {
            assert!(!is_publisher(lookalike, "Skutta Software GmbH"), "{lookalike:?} must not pass");
        }
        assert!(!is_publisher("", ""), "an empty expectation never matches");
    }

    #[cfg(windows)]
    #[test]
    fn an_unsigned_file_has_no_verified_signer() {
        // The test binary is not signed.
        let me = std::env::current_exe().unwrap();
        assert!(signer_organization(&me).is_none());
        assert!(!signed_by(&me, "Skutta Software GmbH"));
        assert!(!signed_by(Path::new(r"C:\no such folder\wallpaper64.exe"), "Skutta Software GmbH"));
    }
}
