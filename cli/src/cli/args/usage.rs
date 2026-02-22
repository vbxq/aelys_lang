pub fn usage() -> &'static str {
    "Usage:
  aelys [flags] <file> [args...]
  aelys run [flags] <file> [args...]
  aelys compile <file>
  aelys asm <file>
  aelys repl [flags]
  aelys version

Flags (any position):
  -h, --help                 Show help
  -v, --version              Show version
  -O<level> or -O <level>    Optimization level: 0,1,2,3, none, basic, standard, aggressive
  -o, --output <path>        Output path (compile/asm)
  --stdout                   Print asm to stdout (asm)
  --emit-air                 Print AIR instead of compiling (compile)
  --backend <vm|llvm>        Compile backend (compile, default: vm)
  --emit-llvm-ir             Emit LLVM IR to <source>.ll (compile + backend llvm)
  -ae.<k>=<v>                VM option (e.g., -ae.max-heap=64M)
  --ae-<k>=<v>               VM option (e.g., --ae-max-heap=64M)
  --allow-caps=<list>        Allow native capabilities (comma-separated)
  --deny-caps=<list>         Deny native capabilities (comma-separated)
  --dev                      Enable dev features (hot reload)

Warning flags:
  -Wall                      Enable all warnings
  -Werror                    Treat warnings as errors
  -W<category>               Enable specific category (inline, unused, deprecated, shadow)
  -Wno-<category>            Disable specific category

Examples:
  aelys main.aelys -O2 --ae-trusted=true
  aelys main.aelys -O2 '-ae.trusted=true'  (quote in PowerShell)
  aelys run -O3 main.aelys arg1 arg2
  aelys repl -ae.max-heap=1G
  aelys asm main.aelys --stdout
  aelys compile main.aelys -o main.avbc -Wall -Werror
  aelys run program.avbc"
}
