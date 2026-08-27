use std::{fs, path::Path};

use anyhow::{anyhow, Context, Result};

#[cfg(target_os = "macos")]
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    },
};

pub fn write_encrypted(path: &Path, plaintext: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("无法创建微信会话目录：{}", parent.display()))?;
    }
    let ciphertext = protect(plaintext)?;
    fs::write(path, ciphertext).with_context(|| format!("无法保存微信登录状态：{}", path.display()))
}

pub fn read_encrypted(path: &Path) -> Result<Vec<u8>> {
    let ciphertext =
        fs::read(path).with_context(|| format!("无法读取微信登录状态：{}", path.display()))?;
    unprotect(&ciphertext)
}

pub fn remove(path: &Path) -> Result<()> {
    remove_platform_secret();
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("无法清理微信登录状态：{}", path.display()))
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn remove_platform_secret() {}

#[cfg(target_os = "macos")]
fn remove_platform_secret() {
    let _ = delete_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT);
}

#[cfg(windows)]
fn protect(plaintext: &[u8]) -> Result<Vec<u8>> {
    let length = u32::try_from(plaintext.len()).map_err(|_| anyhow!("微信登录状态数据过大"))?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: length,
        pbData: plaintext.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let succeeded = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error()).context("Windows 无法加密微信登录状态");
    }
    copy_and_free(output)
}

#[cfg(windows)]
fn unprotect(ciphertext: &[u8]) -> Result<Vec<u8>> {
    let length = u32::try_from(ciphertext.len()).map_err(|_| anyhow!("微信登录状态数据过大"))?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: length,
        pbData: ciphertext.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let succeeded = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error()).context("Windows 无法解密微信登录状态");
    }
    copy_and_free(output)
}

#[cfg(windows)]
fn copy_and_free(output: CRYPT_INTEGER_BLOB) -> Result<Vec<u8>> {
    if output.pbData.is_null() && output.cbData != 0 {
        return Err(anyhow!("Windows 返回了无效的加密数据"));
    }
    let bytes = if output.cbData == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() }
    };
    if !output.pbData.is_null() {
        unsafe {
            LocalFree(output.pbData.cast());
        }
    }
    Ok(bytes)
}

#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "com.ayecode.wechatfeishustickers";

#[cfg(target_os = "macos")]
const KEYCHAIN_ACCOUNT: &str = "wechat-session";

#[cfg(target_os = "macos")]
fn protect(plaintext: &[u8]) -> Result<Vec<u8>> {
    if set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, plaintext).is_err() {
        let _ = delete_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT);
        set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, plaintext)
            .context("macOS Keychain 无法保存微信登录状态")?;
    }
    Ok(b"keychain:v1".to_vec())
}

#[cfg(target_os = "macos")]
fn unprotect(_ciphertext: &[u8]) -> Result<Vec<u8>> {
    get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .context("macOS Keychain 无法读取微信登录状态")
}

#[cfg(not(any(windows, target_os = "macos")))]
fn protect(_plaintext: &[u8]) -> Result<Vec<u8>> {
    Err(anyhow!("当前系统暂不支持安全保存微信登录状态"))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn unprotect(_ciphertext: &[u8]) -> Result<Vec<u8>> {
    Err(anyhow!("当前系统暂不支持读取微信登录状态"))
}

#[cfg(test)]
mod tests {
    use super::{protect, unprotect};

    #[test]
    #[cfg(windows)]
    fn dpapi_round_trip() {
        let source = b"sticker-relay-wechat-session-test";
        let encrypted = protect(source).expect("protect");
        assert_ne!(encrypted, source);
        assert_eq!(unprotect(&encrypted).expect("unprotect"), source);
    }
}
