use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, Copy, Default)]
pub struct VMCapabilities {
    pub allow_fs: bool,
    pub allow_net: bool,
    pub allow_exec: bool,
}

impl VMCapabilities {
    pub const fn all_enabled() -> Self {
        Self {
            allow_fs: true,
            allow_net: true,
            allow_exec: true,
        }
    }

    pub fn set_all(&mut self, enabled: bool) {
        self.allow_fs = enabled;
        self.allow_net = enabled;
        self.allow_exec = enabled;
    }
}

#[derive(Debug, Clone)]
pub struct VmConfig {
    pub max_heap_bytes: u64,
    pub capabilities: VMCapabilities,
    pub allow_hot_reload: bool,
    pub allowed_caps: HashSet<String>,
    pub denied_caps: HashSet<String>,
}

impl VmConfig {
    pub const DEFAULT_MAX_HEAP_BYTES: u64 = 4 * 1024 * 1024 * 1024;
    pub const MIN_HEAP_BYTES: u64 = 1024 * 1024;

    pub fn new(max_heap_bytes: u64) -> Result<Self, VmConfigError> {
        let config = Self {
            max_heap_bytes,
            capabilities: VMCapabilities::default(),
            allow_hot_reload: false,
            allowed_caps: HashSet::new(),
            denied_caps: HashSet::new(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), VmConfigError> {
        if self.max_heap_bytes < Self::MIN_HEAP_BYTES {
            return Err(VmConfigError::MaxHeapTooSmall {
                value: self.max_heap_bytes,
                min: Self::MIN_HEAP_BYTES,
            });
        }
        Ok(())
    }

    pub fn check_native_capability(&self, capability: &str) -> Result<(), String> {
        // denied takes precedence
        if self.denied_caps.contains(capability) {
            return Err(capability.to_string());
        }
        // If allowed_caps is empty, all capabilities are allowed (unless explicitly denied)
        if self.allowed_caps.is_empty() {
            return Ok(());
        }
        // Otherwise, the capability must be in allowed_caps
        if self.allowed_caps.contains(capability) {
            Ok(())
        } else {
            Err(capability.to_string())
        }
    }

    pub fn check_native_capabilities(&self, capabilities: &[String]) -> Result<(), String> {
        for cap in capabilities {
            self.check_native_capability(cap)?;
        }
        Ok(())
    }

    pub fn allow_all_native_caps(&mut self) {
        self.denied_caps.clear();
    }
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            max_heap_bytes: Self::DEFAULT_MAX_HEAP_BYTES,
            capabilities: VMCapabilities::default(),
            allow_hot_reload: false,
            allowed_caps: HashSet::new(),
            denied_caps: HashSet::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum VmConfigError {
    MaxHeapTooSmall { value: u64, min: u64 },
}

impl fmt::Display for VmConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmConfigError::MaxHeapTooSmall { value, min } => {
                write!(
                    f,
                    "max heap too small: {} bytes (minimum {} bytes)",
                    value, min
                )
            }
        }
    }
}
