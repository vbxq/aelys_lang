pub fn usage() -> &'static str {
    "Usage:
  aelys compile [flags] <file>
  aelys help
  aelys version

Flags:
  -h, --help                 Show help
  -v, --version              Show version
  -O<level> or -O <level>    Optimization level: 0,1,2,3, none, basic, standard, aggressive
  -o, --output <path>        Output path
  --emit-air                 Print AIR instead of compiling
  --emit-llvm-ir             Emit LLVM IR to <source>.ll

Warning flags:
  -Wall                      Enable all warnings
  -Werror                    Treat warnings as errors
  -W<category>               Enable specific category (inline, unused, deprecated, shadow, type)
  -Wno-<category>            Disable specific category

Examples:
  aelys compile main.aelys -O2
  aelys compile main.aelys -o output.exe -Wall -Werror
  aelys compile main.aelys --emit-llvm-ir"
}
