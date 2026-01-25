use aelys_runtime::{VmArgsParsed, parse_vm_args};

pub fn parse_vm_args_or_error(args: &[String]) -> Result<VmArgsParsed, String> {
    parse_vm_args(args).map_err(|err| err.to_string())
}
