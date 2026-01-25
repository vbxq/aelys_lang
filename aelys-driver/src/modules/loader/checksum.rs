use std::path::Path;

// FNV-1a file checksum for native module integrity
pub fn compute_file_checksum(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buffer = [0u8; 8192];
    let mut hash = 0xcbf29ce484222325u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 { break; }
        for &byte in &buffer[..n] {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    Ok(format!("{:016x}", hash))
}
