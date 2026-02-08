# Aelys Language Support for VSCode

Syntax highlighting for Aelys programming language.

## Features

- Syntax highlighting for `.aelys` and `.ae` files
- Keywords: `let`, `mut`, `fn`, `if`, `else`, `while`, `for`, `return`, etc.
- Attributes: `@no_gc`, `@inline`, `@inline_always`
- String interpolation support
- Number literals (decimal, hex, binary, octal)
- Comments (`//`)
- Auto-closing brackets and quotes

## Installation

### From VSIX (Recommended)

Download the latest `.vsix` file from the repository root and install:

**Method 1: Drag & Drop**
- Drag the `.vsix` file into VSCode

**Method 2: Extensions Menu**
- VSCode → Extensions → `...` (top right menu) → "Install from VSIX"
- Select the downloaded file

**Method 3: Command Line**
```bash
code --install-extension aelys-0.1.0.vsix
```

### Manual Installation

Copy this entire folder to your VSCode extensions directory:

- **Windows:** `%USERPROFILE%\.vscode\extensions\aelys-0.1.0\`
- **macOS/Linux:** `~/.vscode/extensions/aelys-0.1.0/`

Then restart VSCode.

## Development

To modify the grammar:

1. Edit `syntaxes/aelys.tmLanguage.json`
2. Reload VSCode (`Ctrl+Shift+P` → "Developer: Reload Window")
3. Test your changes on an `.aelys` file

To package a new version:

```bash
npm install -g @vscode/vsce
vsce package
```

This creates a new `.vsix` file for distribution.
