# QBasic Studio

QBasic Studio is a modern Rust terminal IDE and interpreter for a practical,
QBasic-inspired BASIC dialect. It includes an editor, console, interactive
input, file operations, command-line execution, syntax checking, simple graphics,
and a CI/CD pipeline for reliable releases.

## Highlights

- Terminal IDE with BASIC-aware virtual line numbers, syntax highlighting,
  status bar, console output, clipboard support, dirty-file tracking,
  save/open/new workflows, and quit confirmation.
- IDE runs open in a separate program window so a stuck or crashed BASIC program
  can be closed without taking the editor down.
- Command-line modes for running programs and validating syntax in scripts or CI.
- Interpreter support for variables, typed declarations, arrays, user-defined
  records, constants, labels, line numbers, subroutines, functions, loops,
  conditionals, `SELECT CASE`, `DATA`/`READ`/`RESTORE`, file I/O, and core
  string/math functions.
- Graphics support for common QBasic-style drawing commands through a separate
  `minifb` window when running from the IDE.
- Automated quality checks and tag-based release packaging for Windows, macOS,
  and Linux through GitHub Actions.

## Requirements

- Rust stable toolchain.
- A terminal that supports alternate-screen applications.
- Linux graphics builds may require X11 development packages. The included
  GitHub workflow installs them automatically on Ubuntu runners.

## Quick Start

```powershell
cargo run
```

Run an example from the command line:

```powershell
cargo run -- run examples\control_flow.bas
```

Validate a file without executing it:

```powershell
cargo run -- check examples\data_read_restore.bas
```

Build an optimized binary:

```powershell
cargo build --release
```

## Command Line

```text
qbasic_interpreter                 Launch the terminal IDE
qbasic_interpreter <file.bas>      Run a BASIC program
qbasic_interpreter run <file.bas>  Run a BASIC program
qbasic_interpreter check <file>    Parse and validate a program
qbasic_interpreter edit [file]     Open the IDE, optionally with a file
```

## IDE Shortcuts

| Shortcut | Action |
| --- | --- |
| `F5`, `Ctrl+R` | Run the current program in a separate window |
| `F6` | Close the running program window |
| `F2`, `Ctrl+S` | Save |
| `F12` | Save as |
| `F3`, `Ctrl+O` | Open |
| `Ctrl+N` | New program |
| `Ctrl+L` | Clear console |
| `Ctrl+C`, `Ctrl+X`, `Ctrl+V` | Copy, cut, paste |
| `Esc`, `Ctrl+Q` | Quit, with confirmation when needed |

The IDE gutter shows BASIC line numbers in increments of 10. When you run from
the IDE, lines without an explicit BASIC number are executed with those virtual
numbers, so targets like `GOTO 20` work without manually typing every number.

## Language Support

QBasic Studio focuses on the most useful parts of classic QBasic while keeping
runtime behavior predictable:

- Statements: `PRINT`, `LET`, `INPUT`, `LINE INPUT`, `IF/THEN/ELSE`,
  `SELECT CASE`, `FOR/NEXT`, `WHILE/WEND`, `DO/LOOP`, `GOTO`, `GOSUB`,
  `ON ... GOTO/GOSUB`, `RETURN`, `END`, `EXIT`, `DIM`, `REDIM`, `ERASE`,
  `CONST`, `TYPE`, `SUB`, `FUNCTION`, `DECLARE`, `CALL`, `DATA`, `READ`,
  `RESTORE`, `OPTION BASE`, `DEFINT`/`DEFLNG`/`DEFSNG`/`DEFDBL`/`DEFSTR`,
  `RANDOMIZE`, `SLEEP`, `BEEP`, `SWAP`, `CLEAR`, `STOP`, `SYSTEM`.
- File I/O: `OPEN`, `CLOSE`, `INPUT #`, `PRINT #`, `GET`, `PUT`, and random
  record access.
- Console and graphics: `LOCATE`, `SCREEN`, `CLS`, `COLOR`, `PSET`, `LINE`,
  `CIRCLE`, `PAINT`.
- Operators: arithmetic, integer division, exponentiation, comparison, logical
  operators, and string concatenation.
- Built-ins: `LEN`, `MID$`, `LEFT$`, `RIGHT$`, `INT`, `FIX`, `ABS`, `SQR`,
  `RND`, `STR$`, `CSTR$`, `VAL`, `CHR$`, `ASC`, `UCASE$`, `LCASE$`,
  `LTRIM$`, `RTRIM$`, `TRIM$`, `INSTR`, `SIN`, `COS`, `TAN`, `ATN`, `LOG`,
  `EXP`, `SGN`, `TIMER`, `SPACE$`, `SPC`, `TAB`, `STRING$`, `HEX$`, `OCT$`,
  `CINT`, `CLNG`, `CSNG`, and `CDBL`.

Function-like built-ins are called with parentheses, for example `RND()` and
`TIMER()`.

## Examples

The `examples/` directory contains small programs that exercise the main
features:

- `hello.bas` - interactive console input.
- `control_flow.bas` - loops, conditionals, and functions.
- `data_read_restore.bas` - sequential data records and restore behavior.
- `graphics.bas` - drawing commands for the IDE graphics window.

## Development

Use the same commands locally that CI runs:

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Format code before committing:

```powershell
cargo fmt
```

## CI/CD

The workflow in `.github/workflows/ci-cd.yml` provides:

- Pull request and branch checks on Windows, macOS, and Linux.
- Rust formatting, clippy linting, unit tests, and example syntax validation.
- Release builds for all three operating systems when a tag like `v0.3.1` is
  pushed.
- Packaged release archives with SHA-256 checksum files.
- Automatic GitHub Release creation or asset replacement for version tags.

Dependency maintenance is configured in `.github/dependabot.yml` for Cargo and
GitHub Actions updates.

## Release Process

1. Update `Cargo.toml` with the next version.
2. Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and
   `cargo test --all-targets`.
3. Commit the change and push it.
4. Create and push a version tag:

```powershell
git tag v0.3.1
git push origin v0.3.1
```

GitHub Actions will build and publish the release assets automatically.

## Project Structure

```text
src/main.rs         CLI and terminal IDE
src/lexer.rs        Tokenizer for the BASIC dialect
src/parser.rs       AST and parser
src/interpreter.rs  Runtime evaluator and tests
src/graphics.rs     Shared graphics buffer and drawing primitives
examples/           Runnable BASIC sample programs
.github/            CI/CD and dependency automation
```

## Notes

This project is QBasic-inspired rather than a byte-for-byte clone of Microsoft
QBasic. It aims to make classic BASIC programs pleasant to write, run, and test
in a modern terminal workflow.
