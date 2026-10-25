//! Execution-host secrets. Persistent values use macOS Keychain exclusively;
//! no command arguments, serialized secrets or plaintext fallback exist here.
use crate::{
    error::{Error, Result},
    evidence::AttestationKey,
    providers::Credential,
};
use std::fmt;
use uuid::Uuid;

pub struct Secret(Vec<u8>);
impl Secret {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > 64 * 1024 {
            return Err(Error::new(
                "missing_secret",
                "secret must contain 1–65536 bytes",
            ));
        }
        Ok(Self(bytes))
    }
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn credential(&self) -> Result<Credential> {
        let text = std::str::from_utf8(&self.0)
            .map_err(|_| Error::new("missing_secret", "provider credential must be UTF-8"))?;
        Credential::new(text.to_owned())
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            // SAFETY: each byte is valid owned memory. Volatile stores prevent
            // removal of this best-effort clearing; no claim about other copies.
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
    }
}

pub trait SecretStore: Send + Sync {
    fn read(&self, name: &str) -> Result<Secret>;
    /// Setup/reconnection only, after provider identity is verified.
    fn save(&self, name: &str, value: &Secret) -> Result<()>;
}
pub struct Keychain {
    runtime_id: Uuid,
}
impl Keychain {
    pub fn new(runtime_id: Uuid) -> Result<Self> {
        if runtime_id.is_nil() {
            return Err(Error::invalid(
                "Keychain requires registered runtime identity",
            ));
        }
        Ok(Self { runtime_id })
    }
    fn service(&self) -> String {
        format!("com.kegari.v0/{}", self.runtime_id)
    }
    pub fn load_attestation_key(&self) -> Result<AttestationKey> {
        let value = native::read(&self.service(), "attestation-v1")?;
        if value.bytes().len() != 32 {
            return Err(Error::new(
                "missing_secret",
                "host attestation key has invalid length",
            ));
        }
        AttestationKey::new(
            format!("keychain:{}:attestation-v1", self.runtime_id),
            value.bytes().to_vec(),
        )
    }
    /// Explicit bootstrap only. An existing key is never rotated implicitly.
    pub fn initialize_attestation_key(&self) -> Result<AttestationKey> {
        match self.load_attestation_key() {
            Ok(key) => return Ok(key),
            Err(error) if error.code == "missing_secret" => (),
            Err(error) => return Err(error),
        }
        use ring::rand::SecureRandom;
        let mut bytes = vec![0; 32];
        ring::rand::SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| Error::new("prerequisite", "secure random generator is unavailable"))?;
        native::create(&self.service(), "attestation-v1", &Secret::new(bytes)?)?;
        self.load_attestation_key()
    }
}
fn account(name: &str) -> Result<String> {
    if name.trim().is_empty() || name.len() > 512 || name.chars().any(char::is_control) {
        return Err(Error::invalid("invalid Keychain secret reference"));
    }
    Ok(format!("connection/{name}"))
}
impl SecretStore for Keychain {
    fn read(&self, name: &str) -> Result<Secret> {
        native::read(&self.service(), &account(name)?)
    }
    fn save(&self, name: &str, value: &Secret) -> Result<()> {
        native::save(&self.service(), &account(name)?, value)
    }
}

/// Environment values are transient input only. Setup imports them into
/// Keychain and persists a keychain reference, never the expanded value.
pub fn resolve_reference(
    reference: &str,
    store: &dyn SecretStore,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Secret> {
    if let Some(name) = reference.strip_prefix("keychain:") {
        account(name)?;
        return store.read(name);
    }
    if let Some(name) = reference.strip_prefix('$')
        && !name.is_empty()
        && name.bytes().enumerate().all(|(index, c)| {
            c == b'_' || c.is_ascii_alphabetic() || (index > 0 && c.is_ascii_digit())
        })
    {
        return Secret::new(
            env(name)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| {
                    Error::new(
                        "missing_secret",
                        "declared secret environment input is unavailable",
                    )
                })?
                .into_bytes(),
        );
    }
    Err(Error::invalid(
        "secret input must be a Keychain or whole environment reference",
    ))
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use std::ffi::c_void;
    type Ref = *const c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithBytes(
            allocator: Ref,
            bytes: *const u8,
            len: isize,
            encoding: u32,
            external: u8,
        ) -> Ref;
        fn CFDictionaryCreateMutable(
            allocator: Ref,
            capacity: isize,
            keys: Ref,
            values: Ref,
        ) -> Ref;
        fn CFDictionarySetValue(dictionary: Ref, key: Ref, value: Ref);
        fn CFDataCreate(allocator: Ref, bytes: *const u8, len: isize) -> Ref;
        fn CFDataGetLength(value: Ref) -> isize;
        fn CFDataGetBytePtr(value: Ref) -> *const u8;
        fn CFGetTypeID(value: Ref) -> usize;
        fn CFDataGetTypeID() -> usize;
        fn CFRelease(value: Ref);
        static kCFBooleanTrue: Ref;
        static kCFBooleanFalse: Ref;
    }
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecItemCopyMatching(query: Ref, result: *mut Ref) -> i32;
        fn SecItemAdd(attributes: Ref, result: *mut Ref) -> i32;
        fn SecItemUpdate(query: Ref, attributes: Ref) -> i32;
        static kSecClass: Ref;
        static kSecClassGenericPassword: Ref;
        static kSecAttrService: Ref;
        static kSecAttrAccount: Ref;
        static kSecAttrSynchronizable: Ref;
        static kSecUseAuthenticationUI: Ref;
        static kSecUseAuthenticationUIFail: Ref;
        static kSecReturnData: Ref;
        static kSecMatchLimit: Ref;
        static kSecMatchLimitOne: Ref;
        static kSecValueData: Ref;
    }
    struct Owned(Ref);
    impl Owned {
        fn checked(value: Ref) -> Result<Self> {
            if value.is_null() {
                Err(Error::new("prerequisite", "Keychain allocation failed"))
            } else {
                Ok(Self(value))
            }
        }
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: Owned wraps only a successful CoreFoundation create/copy reference.
            unsafe { CFRelease(self.0) };
        }
    }
    struct Query {
        dictionary: Owned,
        _service: Owned,
        _account: Owned,
    }
    impl Query {
        fn new(service: &str, account: &str) -> Result<Self> {
            // SAFETY: borrowed UTF8 slices stay live for copying; created
            // dictionaries use no retain callbacks and Query owns every dynamic
            // referenced value for longer than any Security API call.
            unsafe {
                let s = Owned::checked(CFStringCreateWithBytes(
                    std::ptr::null(),
                    service.as_ptr(),
                    service.len() as isize,
                    0x08000100,
                    0,
                ))?;
                let a = Owned::checked(CFStringCreateWithBytes(
                    std::ptr::null(),
                    account.as_ptr(),
                    account.len() as isize,
                    0x08000100,
                    0,
                ))?;
                let d = Owned::checked(CFDictionaryCreateMutable(
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                    std::ptr::null(),
                ))?;
                for (key, value) in [
                    (kSecClass, kSecClassGenericPassword),
                    (kSecAttrService, s.0),
                    (kSecAttrAccount, a.0),
                    (kSecAttrSynchronizable, kCFBooleanFalse),
                    (kSecUseAuthenticationUI, kSecUseAuthenticationUIFail),
                ] {
                    CFDictionarySetValue(d.0, key, value);
                }
                Ok(Self {
                    dictionary: d,
                    _service: s,
                    _account: a,
                })
            }
        }
    }
    fn status(value: i32) -> Result<()> {
        match value {
            0 => Ok(()),
            -25300 => Err(Error::new(
                "missing_secret",
                "required Keychain item is missing",
            )),
            -25299 => Err(Error::new(
                "ownership_conflict",
                "Keychain item already exists; preserve its identity",
            )),
            _ => Err(Error::new(
                "authentication_required",
                "Keychain access is unavailable; unlock or authorize the installed application explicitly",
            )),
        }
    }
    pub fn read(service: &str, account: &str) -> Result<Secret> {
        let query = Query::new(service, account)?;
        let mut output = std::ptr::null();
        // SAFETY: query and all referenced values stay alive, output is a valid out-pointer.
        unsafe {
            CFDictionarySetValue(query.dictionary.0, kSecReturnData, kCFBooleanTrue);
            CFDictionarySetValue(query.dictionary.0, kSecMatchLimit, kSecMatchLimitOne);
            status(SecItemCopyMatching(query.dictionary.0, &mut output))?;
            let output = Owned::checked(output)?;
            if CFGetTypeID(output.0) != CFDataGetTypeID() {
                return Err(Error::new(
                    "authentication_required",
                    "Keychain returned an unexpected item type",
                ));
            }
            let len = CFDataGetLength(output.0);
            if !(1..=65536).contains(&len) {
                return Err(Error::new(
                    "missing_secret",
                    "Keychain item has invalid secret length",
                ));
            }
            Secret::new(
                std::slice::from_raw_parts(CFDataGetBytePtr(output.0), len as usize).to_vec(),
            )
        }
    }
    pub fn create(service: &str, account: &str, value: &Secret) -> Result<()> {
        let query = Query::new(service, account)?;
        // SAFETY: bytes and query outlive the call; Security copies the data.
        unsafe {
            let data = Owned::checked(CFDataCreate(
                std::ptr::null(),
                value.bytes().as_ptr(),
                value.bytes().len() as isize,
            ))?;
            CFDictionarySetValue(query.dictionary.0, kSecValueData, data.0);
            status(SecItemAdd(query.dictionary.0, std::ptr::null_mut()))
        }
    }
    pub fn save(service: &str, account: &str, value: &Secret) -> Result<()> {
        match create(service, account, value) {
            Ok(()) => return Ok(()),
            Err(error) if error.code == "ownership_conflict" => (),
            Err(error) => return Err(error),
        }
        let query = Query::new(service, account)?;
        // SAFETY: all dictionary entries are owned for the duration of the call.
        unsafe {
            let data = Owned::checked(CFDataCreate(
                std::ptr::null(),
                value.bytes().as_ptr(),
                value.bytes().len() as isize,
            ))?;
            let update = Owned::checked(CFDictionaryCreateMutable(
                std::ptr::null(),
                0,
                std::ptr::null(),
                std::ptr::null(),
            ))?;
            CFDictionarySetValue(update.0, kSecValueData, data.0);
            status(SecItemUpdate(query.dictionary.0, update.0))
        }
    }
}
#[cfg(not(target_os = "macos"))]
mod native {
    use super::*;
    pub fn read(_: &str, _: &str) -> Result<Secret> {
        Err(Error::new(
            "prerequisite",
            "Kegari v0 requires macOS Keychain",
        ))
    }
    pub fn create(_: &str, _: &str, _: &Secret) -> Result<()> {
        Err(Error::new(
            "prerequisite",
            "Kegari v0 requires macOS Keychain",
        ))
    }
    pub fn save(service: &str, account: &str, value: &Secret) -> Result<()> {
        create(service, account, value)
    }
}
