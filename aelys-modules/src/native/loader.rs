// FFI native module loading
// FIXME: consider dlopen flags for lazy vs eager binding

use aelys_native::{
    AELYS_ABI_VERSION, AelysExport, AelysExportKind, AelysModuleDescriptor, AelysRequiredModule,
};
use libloading::Library;
use std::collections::HashMap;
use std::ffi::CString;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

#[cfg(target_os = "linux")]
use std::os::unix::io::FromRawFd;

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

const MAX_EXPORT_COUNT: u32 = 65536;
const MAX_REQUIRED_MODULE_COUNT: u32 = 65536;
const MAX_CSTR_LEN: usize = 4096;

#[derive(Debug)]
pub enum NativeError {
    Io(std::io::Error),
    Load(libloading::Error),
    MissingDescriptor,
    InvalidDescriptor(&'static str),
    InvalidAbi {
        expected: u32,
        found: u32,
    },
    InvalidExportsHash {
        expected: u64,
        found: u64,
    },
    InvalidUtf8,
    DuplicateExport(String),
    PanicDuringLoad,
    InvalidFunctionPointer {
        export_name: String,
        reason: &'static str,
    },
}

impl std::fmt::Display for NativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NativeError::Io(err) => write!(f, "io error: {}", err),
            NativeError::Load(err) => write!(f, "load error: {}", err),
            NativeError::MissingDescriptor => write!(f, "missing module descriptor"),
            NativeError::InvalidDescriptor(msg) => write!(f, "invalid descriptor: {}", msg),
            NativeError::InvalidAbi { expected, found } => {
                write!(
                    f,
                    "abi version mismatch: expected {}, found {}",
                    expected, found
                )
            }
            NativeError::InvalidExportsHash { expected, found } => {
                write!(
                    f,
                    "exports_hash mismatch: expected {}, computed {}",
                    expected, found
                )
            }
            NativeError::InvalidUtf8 => write!(f, "invalid UTF-8 in descriptor"),
            NativeError::DuplicateExport(name) => write!(f, "duplicate export: {}", name),
            NativeError::PanicDuringLoad => write!(f, "panic during native load"),
            NativeError::InvalidFunctionPointer {
                export_name,
                reason,
            } => {
                write!(
                    f,
                    "invalid function pointer for '{}': {}",
                    export_name, reason
                )
            }
        }
    }
}

impl std::error::Error for NativeError {}

impl From<std::io::Error> for NativeError {
    fn from(err: std::io::Error) -> Self {
        NativeError::Io(err)
    }
}

impl From<libloading::Error> for NativeError {
    fn from(err: libloading::Error) -> Self {
        NativeError::Load(err)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NativeExport {
    pub kind: AelysExportKind,
    pub arity: u8,
    pub value: *const std::ffi::c_void,
}

pub struct NativeModule {
    pub name: String,
    pub version: Option<String>,
    pub required_modules: Vec<RequiredModule>,
    pub exports: HashMap<String, NativeExport>,
    pub descriptor: *const AelysModuleDescriptor,
    _lib: Library,
    _embedded: Option<EmbeddedHandle>,
}

#[derive(Debug, Clone)]
pub struct RequiredModule {
    pub name: String,
    pub version_req: Option<String>,
}

pub struct NativeLoader;

impl NativeLoader {
    pub fn new() -> Self {
        Self
    }

    pub fn load_dynamic(&self, name: &str, path: &Path) -> Result<NativeModule, NativeError> {
        // SAFETY: Library::new loads a shared library. This is inherently unsafe
        // because we're executing foreign code. We validate the ABI version and
        // exports hash before calling any functions from it.
        let lib = unsafe { Library::new(path) }?;
        self.load_library_with_embedded(name, lib, None)
    }

    pub fn load_library(&self, name: &str, lib: Library) -> Result<NativeModule, NativeError> {
        self.load_library_with_embedded(name, lib, None)
    }

    pub fn load_embedded(&self, name: &str, bytes: &[u8]) -> Result<NativeModule, NativeError> {
        let (lib, handle) = load_embedded_library(name, bytes)?;
        self.load_library_with_embedded(name, lib, Some(handle))
    }

    fn load_library_with_embedded(
        &self,
        name: &str,
        lib: Library,
        embedded: Option<EmbeddedHandle>,
    ) -> Result<NativeModule, NativeError> {
        // SAFETY: lib.get looks up a symbol by name. The symbol exists because
        // aelys_native proc macros generate it. We check for null below.
        let symbol: libloading::Symbol<*const AelysModuleDescriptor> =
            unsafe { lib.get(b"aelys_module_descriptor\0") }?;
        let descriptor = *symbol;
        if descriptor.is_null() {
            return Err(NativeError::MissingDescriptor);
        }

        // SAFETY: just checked non-null above. Lifetime tied to lib which we keep alive.
        let descriptor_ref = unsafe { &*descriptor };
        if descriptor_ref.abi_version != AELYS_ABI_VERSION {
            return Err(NativeError::InvalidAbi {
                expected: AELYS_ABI_VERSION,
                found: descriptor_ref.abi_version,
            });
        }

        let expected_size = std::mem::size_of::<AelysModuleDescriptor>() as u32;
        if descriptor_ref.descriptor_size < expected_size {
            return Err(NativeError::InvalidDescriptor("descriptor size too small"));
        }

        let module_name = if descriptor_ref.module_name.is_null() {
            name.to_string()
        } else {
            cstr_to_string(descriptor_ref.module_name)?
        };

        let module_version = if descriptor_ref.module_version.is_null() {
            None
        } else {
            Some(cstr_to_string(descriptor_ref.module_version)?)
        };

        if descriptor_ref.exports_hash == 0 {
            return Err(NativeError::InvalidDescriptor("exports_hash is missing"));
        }

        let required_modules = read_required_modules(descriptor_ref)?;
        let exports = read_exports(descriptor_ref)?;
        let computed_hash = unsafe {
            aelys_native::compute_exports_hash(descriptor_ref.exports, descriptor_ref.export_count)
        };
        if descriptor_ref.exports_hash != computed_hash {
            return Err(NativeError::InvalidExportsHash {
                expected: descriptor_ref.exports_hash,
                found: computed_hash,
            });
        }

        Ok(NativeModule {
            name: module_name,
            version: module_version,
            required_modules,
            exports,
            descriptor,
            _lib: lib,
            _embedded: embedded,
        })
    }
}

fn validate_function_pointer(name: &str, ptr: *const std::ffi::c_void) -> Result<(), NativeError> {
    // Check 1: Non-null (this is also checked in module_loader, but we validate early)
    if ptr.is_null() {
        return Err(NativeError::InvalidFunctionPointer {
            export_name: name.to_string(),
            reason: "function pointer is null",
        });
    }

    // Check 2: Alignment check for function pointers
    // Function pointers should be at least aligned to the size of a pointer
    let ptr_addr = ptr as usize;
    let fn_ptr_align = std::mem::align_of::<aelys_native::AelysNativeFn>();
    if ptr_addr % fn_ptr_align != 0 {
        return Err(NativeError::InvalidFunctionPointer {
            export_name: name.to_string(),
            reason: "function pointer is not properly aligned",
        });
    }

    // Check 3: Address range sanity check
    // The first 4KB (0x1000) is typically unmapped on most operating systems
    // to catch null pointer dereferences. A valid function pointer should not
    // be in this range.
    const MIN_VALID_ADDRESS: usize = 0x1000;
    if ptr_addr < MIN_VALID_ADDRESS {
        return Err(NativeError::InvalidFunctionPointer {
            export_name: name.to_string(),
            reason: "function pointer is in reserved address range (likely corrupted)",
        });
    }

    // Check 4: On 64-bit systems, verify the pointer is in a reasonable range
    // User-space addresses typically don't exceed certain bounds
    #[cfg(target_pointer_width = "64")]
    {
        // On most 64-bit systems, user-space is limited to 48-bit addresses (256TB)
        // or 57-bit with 5-level paging. We use a conservative upper bound.
        const MAX_USER_ADDRESS: usize = 0x0000_7FFF_FFFF_FFFF;
        if ptr_addr > MAX_USER_ADDRESS {
            return Err(NativeError::InvalidFunctionPointer {
                export_name: name.to_string(),
                reason: "function pointer is outside valid user-space address range",
            });
        }
    }

    Ok(())
}

fn read_exports(
    descriptor: &AelysModuleDescriptor,
) -> Result<HashMap<String, NativeExport>, NativeError> {
    if descriptor.export_count == 0 {
        return Ok(HashMap::new());
    }
    if descriptor.exports.is_null() {
        return Err(NativeError::InvalidDescriptor("exports pointer is null"));
    }
    if descriptor.export_count > MAX_EXPORT_COUNT {
        return Err(NativeError::InvalidDescriptor("export_count exceeds limit"));
    }

    // SAFETY: exports ptr validated non-null above, count validated <= MAX_EXPORT_COUNT.
    // The pointer comes from the module's static data, so it outlives this call.
    let exports =
        unsafe { std::slice::from_raw_parts(descriptor.exports, descriptor.export_count as usize) };
    let mut map = HashMap::with_capacity(exports.len());
    for export in exports {
        let name = export_name(export)?;
        if map.contains_key(&name) {
            return Err(NativeError::DuplicateExport(name));
        }

        // Validate function pointers before storing
        if matches!(export.kind, AelysExportKind::Function) {
            validate_function_pointer(&name, export.value)?;
        }

        map.insert(
            name,
            NativeExport {
                kind: export.kind,
                arity: export.arity,
                value: export.value,
            },
        );
    }

    Ok(map)
}

fn read_required_modules(
    descriptor: &AelysModuleDescriptor,
) -> Result<Vec<RequiredModule>, NativeError> {
    if descriptor.required_module_count == 0 {
        return Ok(Vec::new());
    }
    if descriptor.required_modules.is_null() {
        return Err(NativeError::InvalidDescriptor(
            "required_modules pointer is null",
        ));
    }
    if descriptor.required_module_count > MAX_REQUIRED_MODULE_COUNT {
        return Err(NativeError::InvalidDescriptor(
            "required_module_count exceeds limit",
        ));
    }

    // SAFETY: same reasoning as read_exports - validated ptr and bounded count
    let entries = unsafe {
        std::slice::from_raw_parts(
            descriptor.required_modules,
            descriptor.required_module_count as usize,
        )
    };
    let mut modules = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = required_module_name(entry)?;
        let version_req = if entry.version_req.is_null() {
            None
        } else {
            Some(cstr_to_string(entry.version_req)?)
        };
        modules.push(RequiredModule { name, version_req });
    }
    Ok(modules)
}

fn required_module_name(entry: &AelysRequiredModule) -> Result<String, NativeError> {
    if entry.name.is_null() {
        return Err(NativeError::InvalidDescriptor(
            "required module name is null",
        ));
    }
    cstr_to_string(entry.name)
}

fn export_name(export: &AelysExport) -> Result<String, NativeError> {
    if export.name.is_null() {
        return Err(NativeError::InvalidDescriptor("export name is null"));
    }
    cstr_to_string(export.name)
}

fn cstr_to_string(ptr: *const std::ffi::c_char) -> Result<String, NativeError> {
    let bytes = ptr as *const u8;
    let mut len = 0;
    while len < MAX_CSTR_LEN {
        if unsafe { *bytes.add(len) } == 0 {
            break;
        }
        len += 1;
    }
    if len == MAX_CSTR_LEN {
        return Err(NativeError::InvalidDescriptor("string exceeds max length"));
    }
    let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
    std::str::from_utf8(slice)
        .map(|s| s.to_string())
        .map_err(|_| NativeError::InvalidUtf8)
}

fn load_embedded_library(
    name: &str,
    bytes: &[u8],
) -> Result<(Library, EmbeddedHandle), NativeError> {
    #[cfg(target_os = "linux")]
    {
        if let Some(result) = load_memfd_library(name, bytes)? {
            return Ok(result);
        }
    }

    create_temp_library(name, bytes)
}

#[cfg(target_os = "linux")]
fn load_memfd_library(
    name: &str,
    bytes: &[u8],
) -> Result<Option<(Library, EmbeddedHandle)>, NativeError> {
    let c_name = CString::new(name).unwrap_or_else(|_| CString::new("aelys").unwrap());
    let fd = unsafe { libc::memfd_create(c_name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ENOSYS) {
            return Ok(None);
        }
        return Err(NativeError::Io(err));
    }

    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(bytes)?;
    file.sync_all()?;

    let path = format!("/proc/self/fd/{}", fd);
    let lib = unsafe { Library::new(&path) }?;

    Ok(Some((lib, EmbeddedHandle::Memfd(file))))
}

fn create_temp_library(name: &str, bytes: &[u8]) -> Result<(Library, EmbeddedHandle), NativeError> {
    let mut path = std::env::temp_dir();
    let safe_name = sanitize_name(name);
    let unique = format!(
        "aelys-native-{}-{}-{}",
        safe_name,
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    path.push(unique);

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }

    let mut file = options.open(&path)?;
    file.write_all(bytes)?;
    file.sync_all()?;

    let cleanup_path = path.clone();
    let lib_result = std::panic::catch_unwind(|| unsafe { Library::new(&path) });
    let lib = match lib_result {
        Ok(Ok(lib)) => lib,
        Ok(Err(err)) => {
            let _ = std::fs::remove_file(&cleanup_path);
            return Err(NativeError::Load(err));
        }
        Err(_) => {
            let _ = std::fs::remove_file(&cleanup_path);
            return Err(NativeError::PanicDuringLoad);
        }
    };

    let _ = std::fs::remove_file(&cleanup_path);

    Ok((
        lib,
        EmbeddedHandle::TempFile {
            path: cleanup_path,
            file,
        },
    ))
}

fn sanitize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "module".to_string()
    } else {
        out
    }
}

#[allow(dead_code)]
enum EmbeddedHandle {
    Memfd(std::fs::File),
    TempFile { path: PathBuf, file: std::fs::File },
}

impl Drop for EmbeddedHandle {
    fn drop(&mut self) {
        if let EmbeddedHandle::TempFile { path, .. } = self {
            let _ = std::fs::remove_file(path);
        }
    }
}
