# Contributing Guide

The contribution standard for everyone who writes code here, human or AI agent.

`.clang-format` decides whitespace, wrapping, brace placement and pointer alignment. Run the formatter rather than argue about layout.

---

## Getting Started

1. Fork the repository on GitHub.
2. Clone your fork locally.
3. Build the project with `.\build.bat`.
4. Before submitting changes, format modified C++ files.

---

## Language & Build

|            Item | Standard                                                              |
|-----------------|-----------------------------------------------------------------------|
|   Engine        | Rust, edition 2024, toolchain 1.85 or later (`Cargo.toml`)            |
|   Server        | C++23 (`CMAKE_CXX_STANDARD 23`)                                       |
|        Compiler | MSVC 2022 (primary)                                                   |
|    Build system | Cargo for the engine, CMake 3.20 or later for the server              |
| Package manager | cargo (engine), vcpkg (server)                                        |
|         Testing | `cargo test` for the engine, Google Test with CTest discovery for C++ |

The style rules in this guide are the **C++** rules. Rust code follows whatever `cargo fmt` produces and must be clean under `cargo clippy --all-targets --release -- -D warnings`.

---

## Core Principle

Prefer code that is:

- easy to read
- easy to review
- easy to debug
- easy to extend safely

Consistency matters more than personal preference.

---

## What `clang-format` Already Covers

The repository formatter already defines the mechanical style for C++ source, including:

- indentation and tabs/spaces
- brace layout
- constructor initializer formatting
- pointer/reference alignment
- spacing around control statements
- include sorting behavior
- wrapping/alignment of arguments, parameters, and comments

Do not restate or fight these rules in review. Run the formatter and move on.

---

## Naming Conventions

Examples below are real identifiers from `src/`. When in doubt, match the file
you are editing rather than this table.

| Element                          | Style                                       | Examples                                                  |
|----------------------------------|---------------------------------------------|-----------------------------------------------------------|
| Files                            | PascalCase, paired `.hpp` / `.cpp`          | `Logger.hpp`, `SalmaEngine.cpp`                           |
| Classes                          | PascalCase                                  | `Logger`, `SalmaEngine`, `Mo2Controller`                  |
| Structs (plain data)             | PascalCase                                  | `FileOperation`, `UploadedFile`                           |
| Enums                            | PascalCase                                  | `enum class FileOpType`, `enum class PluginType`          |
| Enum values                      | PascalCase                                  | `PluginType::Required`, `FileOpType::Folder`              |
| Functions / Methods              | snake_case                                  | `install_mod()`, `is_safe_destination()`                  |
| Namespaces                       | lowercase, run together                     | `mo2core`, `mo2server`                                    |
| Local variables                  | snake_case                                  | `exit_code`, `env_block`, `bind_addr`                     |
| Parameters                       | snake_case                                  | `const std::filesystem::path& script_path`                |
| Class member variables           | snake_case with a trailing `_`              | `scan_job_`, `cache_mutex_`, `static_dir_`                |
| Struct fields (plain data)       | snake_case, no prefix and no suffix         | `temp_path`, `original_extension`, `document_order`       |
| Constants                        | `k` + PascalCase                            | `kMaxUploadBytes`, `kScriptTimeoutMs`, `kMaxRotatedFiles` |
| File-local constants             | `k` + PascalCase, `static constexpr`        | `static constexpr int kMaxRetries = 3;`                   |
| Type aliases                     | PascalCase via `using`                      | `using FnInferSelections = const char*(__cdecl*)(...);`   |
| Global mutable state             | avoided (no `g_` prefix in headers)         | file-scope statics in the `.cpp`; see **Scoping & Lifetime** |

The trailing-underscore member style and the `k` constant prefix are the two
that trip up a contributor arriving from a `m_`-prefixed, `UPPER_SNAKE_CASE`
codebase. There is no `m_` anywhere in `src/`.

### Struct fields (plain data)

Plain structs used as passive data holders (value types, POD aggregates) use **`snake_case`** fields with no prefix and no trailing underscore (no methods, no invariants):

```cpp
struct UploadedFile
{
    std::string filename;            ///< Original name from `Content-Disposition`. Not sanitized.
    std::string temp_path;           ///< Absolute temp-file path. Empty on any failure.
    std::string original_extension;  ///< Sanitized extension with the dot, for example `.7z`.
};
```

The trailing `_` is reserved for the private members of behavior-bearing **classes**; plain-data structs never carry it. That suffix is the only mark separating the two, so adding it to a struct field, or dropping it from a class member, silently moves the reader into the wrong category.

### Prefer named data over positional data

Prefer small structs with named fields over `std::pair`, `std::tuple`, or multi-value conventions that rely on positional meaning.

Use tuples only when the meaning is already obvious and local.

---

## Source File Organization

### Header files

Every header starts with `#pragma once`. Do not use include guards.

```cpp
#pragma once
```

### Include order

Group includes in this order, separated by a blank line between groups:

1. Corresponding header (`.cpp` files only)
2. Project headers
3. External / third-party headers
4. Standard library headers

Keep includes minimal, explicit, and local to actual usage.

### Forward declarations

Prefer including the real dependency over relying on forward declarations.

Use forward declarations only when they provide a clear benefit, such as breaking a circular dependency or reducing heavy include cost in a stable interface.

Do not use forward declarations that make ownership, inheritance, or required type completeness unclear.

### Inline definitions

Only define functions inline when they are genuinely small and benefit from being in the header.

Long or non-trivial implementations belong in `.cpp` files.

---

## Scoping & Lifetime

### Namespaces

* Never use `using namespace` in header files.
* In `.cpp` files, limit `using` directives to narrow scopes.
* Prefer `using std::string;` over `using namespace std;` when local aliasing is helpful.

### Local variables

* Declare variables in the narrowest practical scope.
* Initialize variables when declared.
* Do not separate declaration from first meaningful value unless there is a clear reason.
* Prefer loop-local variables inside the loop statement.

### Internal linkage

Functions, constants, and helpers used only within one translation unit should have internal linkage.

Prefer an unnamed namespace in `.cpp` files for file-local helpers.

```cpp
namespace
{
    float ComputeWeight(float x)
    {
        return x * x;
    }
}
```

### Static and global storage

Avoid non-trivial global state.

Rules:

* prefer `constexpr` or `constinit` where applicable
* prefer function-local statics over namespace-scope mutable singletons
* objects with static storage duration should be trivially destructible unless there is a strong reason otherwise
* global strings should usually be string literals or `std::string_view`, not dynamically initialized `std::string`

---

## Control Flow

### Always use braces

Use braces for all control-flow bodies, even single statements.

This is a project rule even if formatting could make a one-liner look acceptable.

### Prefer early exits

Reduce nesting when possible:

* return early on invalid state
* continue early in loops
* keep the main path visually obvious

### Switch statements

* Prefer `enum class` over unscoped enums.
* Handle all enumerators explicitly when practical.
* Use `default` only when it is actually desired behavior, not as a way to suppress missing-case thinking.

---

## Classes & Types

### Struct vs. class

Use `struct` for passive data containers with public fields and no invariants.

Use `class` for types with invariants, encapsulation, ownership, or behavior.

### Constructors

* Avoid doing heavy work in constructors when failure is possible.
* Avoid virtual dispatch in constructors and destructors.
* If initialization can fail meaningfully, prefer a factory or an `Initialize()` step.

### Explicit conversions

Mark single-argument constructors and conversion operators `explicit` unless implicit conversion is clearly intended and beneficial.

Copy and move constructors are exempt.

```cpp
explicit Texture(const std::string& path);
```

### Copy/move behavior

Be explicit about ownership semantics.

A type should clearly communicate whether it is:

* copyable
* move-only
* neither copyable nor movable

Delete or default the relevant operations intentionally. Do not leave semantics ambiguous.

```cpp
Texture(Texture&& other) noexcept;
Texture& operator=(Texture&& other) noexcept;
Texture(const Texture&) = delete;
Texture& operator=(const Texture&) = delete;
```

### Operator overloading

Only overload operators when behavior is obvious, conventional, and unsurprising.

Do not overload operators with unusual semantics. Never overload:

* `&&`
* `||`
* `,`
* unary `&`

---

## Functions

### Prefer clear interfaces

* Prefer return values over output parameters.
* Keep parameter lists short and meaningful.
* Put inputs before outputs.
* Prefer strong, descriptive types over ambiguous booleans or loosely related parameter packs.

### Parameter guidance

* cheap input values: pass by value
* non-cheap input values: pass by `const T&`
* output or in/out values: pass by `T&`
* optional input: `const T*` or `std::optional<T>`
* optional output: `T*`

Use raw pointers to express optionality or non-ownership, not ownership transfer.

### Boolean parameters

Avoid multiple boolean parameters in one function signature.

This is hard to read:

```cpp
CreateWidget(true, false, true);
```

Prefer an options struct, enum flags, or separate functions when intent is not obvious.

### Function size

Keep functions focused.

A function that needs multiple screens, many nested branches, or several unrelated responsibilities should usually be split.

---

## Ownership & Resource Management

### Ownership must be obvious

Use types to communicate ownership.

* `std::unique_ptr` for exclusive ownership
* `std::shared_ptr` only when shared lifetime is genuinely required
* raw pointers and references for non-owning access
* references when null is not valid
* pointers when null is a meaningful state

### `std::shared_ptr` is not a default

Use `std::shared_ptr` only with clear justification. Shared ownership makes lifetime harder to reason about and can hide architecture problems.

Be especially careful about cycles.

### RAII first

Prefer RAII-based resource management over manual acquire/release patterns.

If a type owns a resource, make cleanup automatic and local to the type.

---

## Error Handling

### Choose one clear strategy per API

Use the most suitable mechanism for the layer:

* assertions for programmer errors and impossible states
* return values / `std::optional` / expected-style patterns for normal recoverable failure
* exceptions only where the project or subsystem explicitly uses them

Do not mix multiple error-handling strategies in the same small API without a good reason.

### Assertions

Use assertions to document invariants and programmer assumptions, not user-driven runtime conditions.

An assertion should mean: if this fails, the code is wrong.

---

## Comments & Documentation

### How to write the prose

These four rules hold everywhere: C++ comments, Rust doc comments, Python docstrings, TypeScript and Markdown.

1. **What, why, how, in that order, and only as much as is needed.** What the thing does in one line. Why it works this way: the ordering that matters, the cap and the reason for it, the guarantee another module leans on. How, only when the mechanism is not visible in the code. Never restate the signature; a parameter's name and type already say what they say, so add the units, the range, whether null is allowed, or nothing at all.
2. **No shouting.** Prose is lowercase. Capitals are for acronyms, identifiers copied from the code, and the Rust `// SAFETY:` marker. If a point needs emphasis, put it first in the sentence.
3. **No archaeology.** The reader does not care where the code came from. Do not write who ported it, which task number carried it, or what an earlier implementation did. A deliberate oddity still has to be marked deliberate, but as a present-tense constraint: name the behavior, say what breaks if someone "fixes" it, and point at `PARITY-NOTES.md` when the long version lives there. This rule bans narrating where code came from; it does not oblige you to invent a forward-looking reason when none exists. If a construct exists only because it reproduces behavior recorded in `PARITY-NOTES.md`, say what it does, say what changing it would break in observable terms, and point at the note. A pointer is not archaeology. Inventing a rationale the code does not support is worse than a bare pointer, because the next maintainer will check it, find it false, and "fix" the construct.
4. **Concise.** Short sentences, active voice, one idea each. One term per concept per file. A set of cases wants a table, a flow wants a diagram. Long is fine when every line carries a fact; long is not fine when it is one fact phrased three times.

Never delete an ASCII diagram, mermaid diagram, table or formula, and never flatten one back into prose. Never drop a contract: preconditions, failure values, who frees what, thread safety, ordering, units, ranges, caps, encodings.

### Rust documentation comments

`src/*.rs` is the larger half of the codebase and it does **not** follow the doxide rules below. It uses `//!` module docs and `///` item docs, rendered by `rustdoc`.

Four constraints that do not transfer from the C++ side:

| Constraint | Why it matters |
|---|---|
| No mermaid, no MathJax | `build.bat` runs a plain `cargo doc --no-deps --release --document-private-items`. Nothing injects `--html-in-header`, so a ```` ```mermaid ```` fence and a `$$ ... $$` block both render as literal text. Draw with ASCII inside a ```` ```text ```` fence; every `.rs` file in `src/` already does. |
| Intra-doc links are denied on breakage | `Cargo.toml` sets `[lints.rustdoc]` with `broken_intra_doc_links = "deny"` and `redundant_explicit_links = "deny"`. Every ``[`Symbol`]`` you write must resolve, and the explicit ``[`X`](X)`` form is an error, not a warning. `cargo doc` is a build gate: `build.bat` step 7 treats a rustdoc lint failure as fatal, the same as clippy. |
| Private items are published | `--document-private-items` means a `///` on a private fn ships to the page. Write it for a reader, not as a scratch note. |
| Module names are load-bearing | Modules mirror the C++ translation units they were ported from, snake_cased (`FomodIRParser.cpp` -> `fomod_ir_parser.rs`). `PARITY-NOTES.md` cross-references resolve through those names, so renaming a module breaks the record. |

Everything in **How to write the prose** above still applies: what/why/how, lowercase, no archaeology, concise, and never drop a contract. The `// SAFETY:` marker is the one place capitals are correct in Rust prose.

### C++ documentation comments

The rest of this section applies to the C++ server in `src/*.{hpp,cpp}` only.

Documentation comments are written for **doxide 0.9.0**, which parses `@`-command Javadoc comments and emits Markdown. Headers and sources use deliberately different styles, and a de-sync is easy to introduce, so check the split table below before adding a block. `clang-format` does not reflow comment text, so keep every comment body inside the column limit yourself.

### The generator is doxide, not Doxygen

salma renders the C++ half of its API reference with `doxide build` -> `scripts/_clean_docs.py` -> `mkdocs build`. doxide understands a subset of the Doxygen command set, and anything outside that subset fails silently: doxide copies it into the generated Markdown as literal text, so the command name shows up on the published page.

Several commands a Doxygen-trained author reaches for first are therefore banned here even though Doxygen accepts them. Write the Markdown equivalent, which doxide passes through untouched and MkDocs Material renders.

| Doxygen habit | What doxide does with it | Write this instead |
|-----------------------------------|------------------------------------------------------------------|------------------------------------------|
| `@par Title`                      | prints the literal text `@par Title` on the page                  | `## Title` (file/type block) or `**Title**` (function block) |
| `@f[ ... @f]`                     | prints verbatim; no math renders                                  | `$$ ... $$`                              |
| `@f$ ... @f$`                     | prints verbatim; no math renders                                  | `$ ... $`                                |
| `@name` with `@{` / `@}`          | prints verbatim, and corrupts the next member's description cell   | a `## Group name` heading in the type block that names the members |
| `@code{.cpp}` / `@endcode`        | emits a fence tagged `{.cpp}`, which is not a valid lexer name    | a fenced ```` ```cpp ```` block          |
| `@ref Target`                     | mangled: swallows the words after it as the link label            | `` `Target` ``                           |

`@brief` and `@author` also leak onto the page, and are required anyway: `scripts/_clean_docs.py` strips both before MkDocs runs. Keep them in existing comments.

These render correctly and may be used freely: `@ingroup`, `@param`, `@return`, `@tparam`, `@note`, `@warning`, `@see`, `@pre`, `@post`, `@p`, `@c`, `@a`, `@struct`, `@class`, `@enum`, `@verbatim` / `@endverbatim`, and trailing `///<` member docs. So do Markdown headings, Markdown tables, fenced code blocks, ```` ```mermaid ```` fences (`mkdocs.yml` registers the superfence), `$...$` and `$$...$$` math (`pymdownx.arithmatex` is wired), and `!!! note "Title"` admonitions.

Placement follows from how doxide nests its output. A `## Heading` in a file or type block renders at page top level and is correct. A function block's body is nested inside a `!!! function` admonition, so use a bold lead-in such as `**Ordering**` there instead, and keep mermaid fences and `$$` blocks out of function blocks entirely.

### The header/source split (the core rule)

|                       | `.hpp`                                                  | `.cpp`                                            |
|-----------------------|---------------------------------------------------------|---------------------------------------------------|
| Doc-comment styles    | `/** ... */` blocks, `///` one-liners, `///<` trailing  | plain `//` only                                   |
| Doxygen commands      | yes (`@brief`, `@param`, `@ingroup`, ...)               | none, with one exception: `@author`               |
| `@author`             | file/type-level, `[NAME] (https://github.com/[USER])`   | optional `// @author <Name> (<url>)` line         |

### General comment rule

Comment the reason, the constraint, or the non-obvious behavior. Do not comment what the code already says. ASCII diagrams and worked-example traces in algorithm-heavy code stay: they are house style, not clutter.

Bad:

```cpp
count++; // Increment count
```

Better:

```cpp
count++; // Includes the sentinel slot reserved during parsing.
```

### Where documentation lives

* Header files: documentation for the public-facing API (types, functions, members).
* Source files: implementation notes for non-obvious logic.

---

### Header (`.hpp`) documentation

#### Block vs. one-line form

* A doc comment spanning more than one line is a Javadoc block: `/**` on its own line, a leading ` * ` on every continuation line, a bare ` *` for blank separator lines, and ` */` to close.
* A doc comment that fits on one physical line uses `///`, for example a lone `/// @brief ...` above a simple declaration.
* Headers use only these two styles. The `//!` and `/*! */` "bang" variants are not used.

```cpp
/// @brief Reset the buffer to its empty state.
void clear();

/**
 * @brief Resize the buffer, preserving existing contents.
 * @param new_size   Desired capacity, in elements.
 * @param zero_fill  Whether newly added slots are zero-initialized.
 */
void resize(std::size_t new_size, bool zero_fill);
```

#### File / type header block

The block documenting a header's primary type (or namespace) uses a **fixed tag order**:

1. **Kind tag** - `@struct Name`, `@class Name`, or `@enum Name`. Present when the header defines one primary type; **omit** it for namespace / free-function headers, which lead with `@brief`.
2. `@brief` - a one-line summary ending in a period.
3. `@author [NAME] (https://github.com/[USER])` - the same attribution string everywhere. **File / type-level only**; never repeated on a function, method, or member.
4. `@ingroup <Module>` - one of the modules your project defines (see below).
5. a blank ` *`, then prose.

```cpp
/**
 * @struct Color
 * @brief 8-bit RGBA color.
 * @author [NAME] (https://github.com/[USER])
 * @ingroup <Module>
 *
 * Plain data struct: a flat aggregate with no invariants, usable directly as
 * a value type. Blending helpers live in the free functions in ColorMath.hpp.
 *
 * @see ColorMath
 */
```

A namespace / free-function header drops the kind tag and leads with `@brief`:

```cpp
/**
 * @brief Pure, dependency-free 2D vector math helpers.
 * @author [NAME] (https://github.com/[USER])
 * @ingroup <Module>
 *
 * Each function is a stateless free function operating on plain values, with
 * no global or GPU state.
 */
```

Do not use `@file`; file identity stays implicit.

#### Modules (`@ingroup`)

Every documented entity is grouped under a module with `@ingroup <Module>`. Groups are declared in **`doxide.yml`**, not in a header: `@defgroup` and `@addtogroup` are unsupported and must not appear in the sources. Adding a group therefore takes three edits, and all three are required:

1. the `groups:` entry in `doxide.yml` (name, title, one-line description),
2. at least one `@ingroup <Name>` in `src/*.hpp`,
3. the matching `nav:` entry in `mkdocs.yml`.

Step 2 says `.hpp` and means it. `doxide.yml`'s `files:` list also scans `src/*.cpp`, but the source rule below forbids every Doxygen command in a `.cpp` except `@author`, so a group tagged from a `.cpp` would satisfy doxide while breaking the header/source split. No `.cpp` in `src/` carries a command today; keep it that way.

Skipping step 2 fails silently. doxide generates nothing for a group with no members, so the `mkdocs.yml` nav entry points at a page that never appears. Note also that a **namespace cannot carry `@ingroup`**: doxide warns and ignores it, so tag the individual types and free functions instead.

Choose the module by **subsystem role, not filename**: a request helper belongs to the server module even when its filename names the feature it serves rather than the module.

`docs/` is regenerated but never cleaned automatically. When you change groups or nav, wipe `docs/` first (keeping `docs/main.html`) and rebuild, or a stale page will satisfy a nav entry that a fresh clone cannot.

#### Function / method documentation

A `/** */` block: `@brief` first (it may reference a parameter with `@p param`, or another symbol as `` `Symbol` ``), a blank line, prose, then `@param` (name plus description, continuation lines aligned under the description) and `@return`. Functions carry no `@author`. The spelling is `@return`, never `@returns`.

Function blocks carry the whole contract, not just the parameter list. State what the signature does not: preconditions, postconditions, what is thrown or returned on failure, whether a pointer may be null, who owns a returned resource and how long it lives, thread safety, whether the call blocks, what I/O it performs, and units, ranges, encodings and ordering guarantees. Spell units and ranges out ("bytes", "milliseconds", "in the range [0, 1]").

```cpp
/**
 * @brief Linearly interpolate between @p a and @p b by @p t.
 *
 * @p t is clamped to [0, 1], so values outside that range saturate to the
 * nearest endpoint.
 *
 * @param a  Start value, returned when @p t is 0.
 * @param b  End value, returned when @p t is 1.
 * @param t  Blend factor in [0, 1].
 * @return   The interpolated value.
 */
float lerp(float a, float b, float t);
```

#### Members and enum values

Document a struct field or enumerator with a trailing `///<` when the text fits on the member's line. `///<` is the only trailing style this guide uses; not `/**< */`, `//!<`, or `/*!< */`.

```cpp
struct Color
{
    std::uint8_t r{0};    ///< Red channel.
    std::uint8_t g{0};    ///< Green channel.
    std::uint8_t b{0};    ///< Blue channel.
    std::uint8_t a{255};  ///< Alpha channel (255 = opaque).
};

enum class LogLevel
{
    Debug,    ///< Verbose developer diagnostics.
    Info,     ///< Normal operational messages.
    Warning,  ///< Recoverable problems worth attention.
    Error     ///< Failures that abort the current operation.
};
```

When a field description is too long for a trailing `///<`, put leading `///` lines above the member instead. This is the one place a `///` comment may span multiple lines. An individual field or enumerator never takes a `/** */` block.

```cpp
/// Master enable flag. When false the renderer skips the entire post-processing
/// chain (blur, bloom, tone-mapping) and presents the raw scene texture
/// unmodified. Toggled at runtime from the developer console.
bool postProcessEnabled{true};
```

#### Grouping related members

Never use `@name` with `@{` ... `@}`. doxide prints all three commands as literal text, and `@}` corrupts the description cell of the member that follows it in the generated member table.

Describe the grouping in the type's own doc block instead, under a `## ` heading that names the members it covers. Each member keeps its own trailing `///<`.

```cpp
/**
 * @class Window
 * @brief Owns the OS window and its client-area geometry.
 * @author [NAME] (https://github.com/[USER])
 * @ingroup <Module>
 *
 * ## Window state
 *
 * `window_`, `width_`, `height_` and `initialized_` move as one unit: they are
 * all set by a successful `create()` and all reset by `destroy()`.
 * `initialized_` gates teardown, so it must stay false until the handle is
 * valid.
 */
class Window
{
    Window* window_ = nullptr;  ///< Owned OS window handle.
    int width_ = 1280;          ///< Client-area width, in pixels.
    int height_ = 720;          ///< Client-area height, in pixels.
    bool initialized_ = false;  ///< Whether creation succeeded (for safe teardown).
};
```

Icon-prefixed headings are house style for these sections, for example `## :material-help: Thread Safety`. Reuse an icon the repository already uses for the same concept rather than inventing one.

#### Command vocabulary

Documentation commands to use where useful:

`@brief`, `@author`, `@ingroup`, `@struct` / `@class` / `@enum`, `@param`, `@return`, `@tparam`, `@pre` / `@post`, `@note` / `@warning`, `@throw`, `@p` / `@c` / `@a` / `@see`, and `@verbatim` / `@endverbatim` for ASCII diagrams.

Everything else is Markdown: `## Title` headings, tables, fenced ```` ```cpp ```` and ```` ```text ```` blocks, ```` ```mermaid ```` diagrams, and `$ ... $` / `$$ ... $$` math.

Never use, because doxide leaks them onto the page: `@par`, `@f[` / `@f]`, `@f$`, `@name`, `@{` / `@}`, `@code` / `@endcode` (in any form), `@ref`.

Never use, because they are unsupported or wrong for this project: `@file`, `@returns` (use `@return`), `@throws` (use `@throw`), `@union`, `@short`, `@defgroup`, `@def`, `@fn`, `@var`, `@internal`, `@namespace`.

---

### Source (`.cpp`) documentation

* `//` line comments only. No `/** */` blocks, no `///` (not even trailing `///<`), and no Doxygen commands (`@brief`, `@param`, `@return`, `@ingroup`, `@par`, `@note`, ...).
* The one exception is an authorship line, written as a plain `// @author <Name> (<url>)`, for example `// @author [NAME] (https://github.com/[USER])`. Keep existing attributions as they stand.
* When moving header prose into a `.cpp`, strip the Doxygen markup: `@p name` -> `name` (or backtick-quote it: `` `name` ``), `@c Buffer{}` -> plain `Buffer{}`, `@ref Foo` -> `Foo`. Backtick-quoting an identifier is the house substitute for `@c` / `@p` inside `//` comments.
* A file-header contract block is optional. Complex files (algorithms, pipelines) carry a top-of-file `//` summary, often with an ASCII diagram or worked example; simpler files go straight from includes to code. Either way, put a one-line `//` intent comment above any function whose purpose is not self-evident.

```cpp
// RingBuffer - fixed-capacity FIFO used by the audio mixer.
//
// @author [NAME] (https://github.com/[USER])
// Reads and writes advance independent cursors modulo the capacity. The
// buffer is treated as full when the write cursor sits one slot behind the
// read cursor, so exactly one slot is always reserved to tell full from empty.
```

```cpp
// Clamp the requested gain to [0, 1] before applying the fade curve.
void SetGain(Channel& channel, float gain)
```

### TODO comments

Use `TODO` only for real follow-up work, not vague reminders.

Make them specific and actionable:

```cpp
// TODO: Replace with a spatial hash once the element count exceeds 10k.
```

---

## API Design Preferences

### Prefer expressive types

Use enums, structs, aliases, and dedicated small types when they make interfaces clearer.

Prefer:

```cpp
struct LoadOptions
{
    bool allowCache;
    bool validateSchema;
};
```

over:

```cpp
bool Load(bool allowCache, bool validateSchema);
```

### Prefer compile-time guarantees

When a rule can be enforced by the type system, `constexpr`, `constinit`, or RAII, prefer that over comments and conventions.

### Avoid hidden work

Functions should not unexpectedly:

* allocate heavily
* block for long periods
* mutate unrelated global state
* transfer ownership invisibly

Make expensive or stateful behavior visible in the API.

---

## Testing

All non-trivial behavior changes should include tests or a clear reason why tests are not practical.

Add or update tests when you change:

* parsing logic
* math/transform code
* serialization
* state machines
* public APIs
* bug fixes with reproducible behavior

A bug fix without a regression test should be the exception, not the norm.

---

## Pull Requests

### Scope

Keep pull requests focused.

Do not mix unrelated refactors, formatting-only churn, feature work, and bug fixes in the same PR unless there is a strong reason.

### What to include

A good PR should explain:

* what changed
* why it changed
* any important tradeoffs
* how it was validated

### Reviewer expectations

Reviewers should prioritize:

* correctness
* maintainability
* API clarity
* architecture fit
* test coverage

Do not spend review time re-litigating rules that are already enforced automatically by tooling.

---

## AI-Assisted Contributions

AI assistance is allowed, but the contributor remains fully responsible for the submitted code.

If you use AI, you must still ensure that the result is:

* correct
* project-consistent
* buildable
* testable
* understandable by a human reviewer

Do not submit generated code you do not understand.

Pay extra attention to:

* hallucinated APIs
* incorrect ownership assumptions
* fake includes
* wrong engine/library types
* missing edge cases
* overly generic comments or documentation