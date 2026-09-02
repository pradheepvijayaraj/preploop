# Contributing to PrepLoop

Thank you for helping improve PrepLoop. Contributions may include bug fixes,
features, tests, documentation, accessibility improvements, question-paper
corrections, and search-quality work.

This guide describes the expected development setup and contribution workflow.
Please read it before opening a pull request.

## Before you begin

- Search existing issues and pull requests before starting overlapping work.
- Open an issue first for a large feature, architecture change, data migration,
  or user-facing redesign.
- Keep each contribution focused on one problem or closely related group of
  changes.
- Do not include unrelated formatting, generated files, or cleanup in the same
  pull request.
- Never commit credentials, updater signing keys, private data, local databases,
  downloaded models, or environment files.
- By submitting a contribution, you agree that it may be distributed under the
  repository's [MIT License](LICENSE).

## Development stack

PrepLoop is a desktop application built with:

- Svelte 5 and SvelteKit
- TypeScript in strict mode
- Tailwind CSS
- Tauri 2
- Rust
- SQLite
- Bun
- Vitest and Testing Library

Question search combines SQLite full-text search with a local Granite embedding
model. The model is downloaded during setup and is not stored in Git.

## Prerequisites

Install the following before cloning the repository:

- [Git](https://git-scm.com/)
- [Bun](https://bun.sh/docs/installation) **1.4.0**
- [Rust through rustup](https://rustup.rs/)
- The [Tauri system dependencies](https://v2.tauri.app/start/prerequisites/)
  for your operating system

The repository pins Rust **1.96.1** with `rust-toolchain.toml`. When rustup is
installed, entering the repository automatically selects that toolchain and its
`rustfmt` and `clippy` components.

Confirm the tools are available:

```sh
git --version
bun --version
rustc --version
cargo --version
```

`bun --version` should report `1.4.0`. `rustc --version` should report `1.96.1`
while run from this repository.

### macOS

Install the Xcode command-line tools:

```sh
xcode-select --install
```

Follow the official Tauri prerequisites if your system still reports a missing
SDK or build tool.

### Windows

Install:

- Microsoft C++ Build Tools with **Desktop development with C++**
- A Windows 10 or Windows 11 SDK
- Microsoft Edge WebView2 if it is not already installed

Use a normal PowerShell or Developer PowerShell terminal. The repository's
Windows ARM64 installer build has additional CI-only compiler configuration;
contributors do not need to reproduce it for ordinary frontend or Rust work.

### Linux

On Debian or Ubuntu, install the native packages used by the project and its CI:

```sh
sudo apt-get update
sudo apt-get install -y \
  build-essential \
  curl \
  file \
  libappindicator3-dev \
  librsvg2-dev \
  libssl-dev \
  libwebkit2gtk-4.1-dev \
  patchelf \
  wget \
  xdg-utils
```

Some distributions provide `libayatana-appindicator3-dev` instead of
`libappindicator3-dev`. For Fedora, Arch, openSUSE, NixOS, and other systems,
use the package list in the official Tauri prerequisites.

## Fork and clone

Fork `utilinlabs/preploop` on GitHub, then clone your fork:

```sh
git clone https://github.com/<your-username>/preploop.git
cd preploop
git remote add upstream https://github.com/utilinlabs/preploop.git
git remote -v
```

Your fork should normally be named `origin`; the main repository should be
named `upstream`.

## First-time setup

From the repository root, run:

```sh
bun run dev:setup
```

This command:

1. Installs JavaScript dependencies from `bun.lock` when they are missing.
2. Uses the lockfile without rewriting dependency versions.
3. Checks whether the local embedding model is present and has the expected
   checksum.
4. Downloads and verifies the model only when it is missing or invalid.

The first setup requires internet access. Later runs skip dependencies and the
model when they are already present and verified.

If dependencies need to be refreshed explicitly, run:

```sh
bun install --frozen-lockfile
```

Do not use npm, Yarn, or pnpm in this repository. Do not replace or regenerate
`bun.lock` unless the dependency change requires it.

## Run PrepLoop locally

Start the desktop application:

```sh
bun run tauri dev
```

This is the preferred development command because it runs the Svelte frontend
inside the native Tauri application.

For frontend-only layout work, you may run:

```sh
bun run dev
```

The browser preview runs at `http://localhost:5173`. Native features such as
local storage commands, desktop menus, and the updater are not fully available
there, so verify native behavior in `bun run tauri dev` before submitting.

## Useful commands

Run these commands from the repository root unless noted otherwise.

| Command                 | Purpose                                                 |
| ----------------------- | ------------------------------------------------------- |
| `bun run dev:setup`     | Install missing dependencies and verify the local model |
| `bun run tauri dev`     | Run the desktop app in development mode                 |
| `bun run dev`           | Run the frontend-only browser preview                   |
| `bun run model:fetch`   | Download or verify the embedding model                  |
| `bun run check`         | Run Svelte and TypeScript diagnostics                   |
| `bun run check:scripts` | Run strict TypeScript diagnostics for Bun scripts       |
| `bun run check:watch`   | Run diagnostics in watch mode                           |
| `bun run test`          | Run the frontend test suite                             |
| `bun run test:coverage` | Run frontend tests with coverage                        |
| `bun run format`        | Format files under `src` with Prettier                  |
| `bun run format:check`  | Check formatting under `src`                            |
| `bun run build`         | Build the production frontend                           |
| `bun run tauri:build`   | Build the local desktop application bundle              |
| `bun run tauri build`   | Build installers supported by the current platform      |

Markdown and other files outside `src` can be checked directly:

```sh
bun x prettier --check README.md CONTRIBUTING.md
```

## Repository map

| Path                            | Purpose                                                     |
| ------------------------------- | ----------------------------------------------------------- |
| `src/routes`                    | Application screens and route-level coordination            |
| `src/lib/components`            | Reusable Svelte UI components and component tests           |
| `src/lib/services`              | Frontend service boundaries and application operations      |
| `src/lib/stores`                | Persisted and session state                                 |
| `src/lib/types`                 | Shared frontend domain and command-response types           |
| `src/lib/constants`             | Fixed catalog and UI definitions                            |
| `src-tauri/src/backend`         | Database, validation, scoring, sessions, and Tauri commands |
| `src-tauri/src/search`          | Lexical, semantic, ranking, and vector-search code          |
| `static/upsc`                   | Bundled UPSC catalog, papers, and taxonomy data             |
| `src-tauri/models/search-index` | Bundled search-index generation                             |
| `scripts`                       | Setup, model, version, and release helpers                  |
| `.github/workflows`             | Pull-request checks and installer builds                    |

## Development workflow

### 1. Update your base branch

Before starting new work:

```sh
git switch main
git fetch upstream
git merge --ff-only upstream/main
```

If your fork's `main` also needs updating:

```sh
git push origin main
```

Do not begin new work with unrelated local modifications. If you already have
work in progress, commit it on its own branch or store it safely before changing
branches.

### 2. Create a focused branch

Use a short branch name in this form:

```text
<type>/<short-description>
```

Examples:

```text
feat/question-bookmarks
fix/updater-retry
docs/contributing-guide
test/session-timeout
data/correct-gs2-2024
```

Recommended branch types are `feat`, `fix`, `docs`, `test`, `refactor`, `data`,
`ci`, and `chore`.

Create the branch:

```sh
git switch -c feat/question-bookmarks
```

### 3. Make and verify the change

- Read the surrounding code before editing.
- Follow existing component, service, store, and backend boundaries.
- Add or update tests for changed behavior.
- Run the smallest relevant checks while working.
- Run the full applicable validation before opening the pull request.
- Review `git diff` and `git status --short` before committing.

### 4. Commit the change

Stage only the files that belong to the change:

```sh
git status --short
git diff
git add <files>
git diff --cached
git diff --cached --check
```

Then create a commit that follows the rules below.

## Coding conventions

### General

- Match the structure and naming of nearby code.
- Prefer small, direct functions with explicit inputs and outputs.
- Keep user-facing copy short, factual, and consistent with the app.
- Avoid unrelated refactors in a bug fix or content correction.
- Explain non-obvious decisions in comments; do not narrate obvious syntax.
- Remove temporary logs, debug flags, screenshots, and experimental files.
- Preserve existing behavior unless the pull request intentionally changes it.
- Treat accessibility, keyboard use, light theme, and dark theme as part of the
  feature rather than follow-up work.

### TypeScript and Svelte

- Use TypeScript and keep strict-mode checks passing.
- Use Svelte 5 runes and the patterns already present in the repository.
- Use `$lib` imports for code under `src/lib`.
- Prefer `import type` for type-only imports.
- Keep route files focused on screen coordination; move reusable behavior into
  components, services, stores, or utilities.
- Name Svelte components and helpers with lowercase kebab-case filenames.
- Place frontend tests beside the code using the `.test.ts` suffix.
- Do not call Tauri's `invoke` directly from arbitrary components. Add commands
  to the typed backend boundary and expose them through a service.
- Keep Rust command payloads and TypeScript types synchronized.
- Clean up event listeners, timers, subscriptions, and pending effects when a
  component is destroyed.
- Guard asynchronous work against stale responses when a screen, session, or
  request can change before completion.
- Persist user changes safely. Optimistic updates must restore the last saved
  state when persistence fails.

### UI and accessibility

- Reuse existing UI primitives before creating another button, dialog, switch,
  checkbox, or portal implementation.
- Preserve the app's restrained visual language and square-edged content
  surfaces unless a design change explicitly calls for something else.
- Verify every UI change in light and dark themes.
- Use semantic elements, associated labels, accessible names, and meaningful
  dialog titles.
- Ensure all actions are reachable by keyboard and show visible focus.
- Manage initial focus and focus return for dialogs.
- Do not use color as the only way to communicate state.
- Respect reduced-motion preferences for non-essential animation.
- Test narrow and minimum-size layouts as well as the normal desktop window.
- Prefer assertions by role and accessible name over brittle raw text or DOM
  structure checks.

### Rust and backend code

- Run `rustfmt` and keep Clippy free of warnings.
- Use `LoopResult` and `LoopError` for backend and Tauri command failures.
- Return user-safe error messages. Log internal library or database details
  locally instead of serializing them to the frontend.
- Validate identifiers, command arguments, imported data, and persisted state at
  their boundaries.
- Use transactions for multi-step database mutations that must succeed together.
- Keep database lock durations short. Snapshot database state first, perform
  model inference or file I/O after releasing the lock, then commit the result in
  a short final transaction.
- Avoid panics in recoverable runtime paths. `unwrap` and `expect` are acceptable
  in tests and for explicit startup invariants, but not as routine error handling.
- Add unit or integration coverage for success, invalid input, persistence
  failure, and stale-state behavior where relevant.

### Search changes

- Do not add query-specific exceptions or hard-coded answers.
- Derive ranking and filtering behavior from general token, field, taxonomy, and
  semantic rules.
- Keep exact lexical or numeric evidence distinct from semantic similarity.
- Preserve the visible distinction between strong matches and related results.
- Test families of queries and corpus-wide behavior, not only the example that
  exposed a problem.
- Record benchmark or evaluation evidence when changing ranking thresholds,
  vector formats, model configuration, or index behavior.

### UPSC paper and taxonomy changes

- Keep `static/upsc/catalog.json` consistent with every bundled paper file.
- Preserve stable paper and question identifiers when correcting content.
- Update question counts, years, paper metadata, taxonomy fields, and content
  versions together.
- Verify marks, negative marks, answer keys, sections, and nested subpart
  breakdowns against the source used for the correction.
- Do not expose answer keys through active test-session payloads.
- Run structural validation across the corpus after changing shared parsing or
  validation logic.
- The bundled search index is generated data. Do not hand-edit its manifest,
  question map, or vector binary.
- If a data or taxonomy change requires a new bundled index, include all index
  files from the same generation and describe the rebuild and validation in the
  pull request.

## Testing conventions

### Frontend tests

- Use Vitest and Testing Library.
- Test visible behavior and accessibility rather than implementation details.
- Add a regression test for every fixed bug when practical.
- Use deterministic state and explicit async waits; do not hide races with
  arbitrary sleep calls.
- Restore mocks, DOM state, timers, and global objects after each test.
- Keep startup and native-boundary coverage current when changing onboarding,
  Settings, startup theming, native command wiring, or the updater.

Run one frontend test file while developing:

```sh
bun x vitest run src/lib/components/example.test.ts
```

Run the full frontend suite:

```sh
bun run test
```

### Rust tests

Place focused unit tests near the module they cover. Use temporary directories
and in-memory databases where possible, and ensure test files are cleaned up.

Run one Rust test:

```sh
cd src-tauri
cargo test --locked test_name
```

Run the complete Rust suite when the verified embedding model is available:

```sh
cd src-tauri
cargo test --locked --lib
```

### Model-backed tests

Search-index rebuild and corpus-recall tests require the verified local model.
Fetch it first:

```sh
bun run model:fetch
```

Then run the two model-backed tests from `src-tauri`:

```sh
cargo test --locked --lib generation_rebuild_embeds_changes_and_reuses_matching_records
cargo test --locked --lib bundled_upsc_corpus_is_fully_searchable_and_similarity_sorted
```

These tests are especially important for changes under `src-tauri/src/search`,
`src-tauri/models`, `static/upsc`, model setup scripts, or Rust dependencies.

## Required checks before a pull request

Run the checks relevant to your change. A normal code change should pass the
same frontend and Rust checks used by CI.

### Frontend checks

```sh
bun run format:check
bun run check
bun run test
bun run build
```

### Rust checks

```sh
cd src-tauri
cargo fmt --all -- --check
cargo test --locked --lib
cargo clippy --locked --all-targets -- -D warnings
```

Run the model-backed tests separately when the change affects their inputs.

### Final repository checks

```sh
git diff --check
git status --short
```

Also check Markdown files changed outside `src`:

```sh
bun x prettier --check README.md CONTRIBUTING.md
```

If a required check cannot run locally, state exactly which check was skipped
and why in the pull request. Do not report an unrun check as passing.

## Commit guidelines

PrepLoop uses Conventional Commit-style subjects:

```text
<type>(<scope>): <summary>
```

Examples:

```text
feat(search): add paper-level query filters
fix(ui): retain focus after closing settings
fix(data): correct GS2 question metadata
test(updater): cover deferred installation
docs(contributing): clarify Linux setup
ci(actions): cache verified model downloads
chore(dev): simplify local setup
```

### Commit types

| Type       | Use for                                                         |
| ---------- | --------------------------------------------------------------- |
| `feat`     | New user-visible behavior                                       |
| `fix`      | Bug fixes and data corrections                                  |
| `docs`     | Documentation only                                              |
| `test`     | Test-only changes                                               |
| `refactor` | Internal restructuring without behavior changes                 |
| `perf`     | Measured performance improvements                               |
| `data`     | Data-only work when `fix(data)` or `feat(data)` is not suitable |
| `build`    | Build system or dependency changes                              |
| `ci`       | GitHub Actions and automation                                   |
| `chore`    | Maintenance that does not fit another type                      |

Common scopes include `ui`, `core`, `search`, `data`, `updater`, `release`,
`actions`, and `dev`. Use a different concise scope when it identifies the
affected area more clearly.

### Commit subject rules

- Use lowercase for the type and scope.
- Write the summary in the imperative mood: `add`, `fix`, `remove`, `preserve`.
- Keep the subject concise, preferably no more than 72 characters.
- Do not end the subject with a period.
- Describe the outcome, not the implementation process.
- Keep one logical change per commit.
- Do not mix formatting-only changes with behavior changes unless formatting is
  required in the same lines.

Use a commit body when the reason, trade-off, migration, or verification is not
obvious from the diff. Explain **why** the change is needed and note important
constraints. Reference an issue with `Closes #123` when applicable.

For a breaking change, add a `BREAKING CHANGE:` footer and explain the migration.

Do not amend, squash, or force-push commits that another contributor may already
be using without coordinating first.

## Pull request guidelines

### Before opening the pull request

- Rebase or merge the latest `upstream/main` into your branch and resolve any
  conflicts.
- Confirm only intended files are changed.
- Review the complete diff yourself.
- Run the applicable check matrix.
- Push the branch to your fork.

```sh
git push -u origin <branch-name>
```

### Pull request title

Use the same format as a commit subject:

```text
fix(search): keep numeric matches exact
```

### Pull request description

Use this structure:

```markdown
## Summary

- What changed
- What users or contributors will notice

## Why

Explain the problem and why this approach was chosen.

## Validation

- [x] `bun run check`
- [x] `bun run test`
- [x] `cargo test --locked --lib`
- [ ] Not run: explain why

## Screenshots

Include before and after images for visible UI changes.

## Notes

List migrations, follow-up work, platform limits, data sources, or reviewer
attention points.

Closes #123
```

### Pull request requirements

- Keep the pull request focused and reviewable.
- Link the relevant issue when one exists.
- Include screenshots or a short recording for visible UI changes in both light
  and dark themes.
- Describe keyboard and accessibility behavior for interactive changes.
- List every check run and its result.
- Explain skipped checks and remaining limitations.
- Include regression coverage for bug fixes.
- Update documentation when user behavior or contributor workflow changes.
- Identify platform-specific effects for macOS, Windows, or Linux.
- For paper corrections, identify the paper, year, question, and verification
  source without copying unnecessary copyrighted material into the PR.
- For search changes, include representative query results and broader regression
  evidence.
- For performance work, include before-and-after measurements and the test
  environment.
- Mark unfinished work as a draft pull request.

CI must pass before merge. The normal pull-request checks cover frontend
formatting, diagnostics, tests, production smoke behavior, production build,
Rust formatting, model-independent Rust tests, and Clippy. Relevant search,
model, and corpus changes also trigger model-backed integration tests.

### Review follow-up

- Respond to each review comment with the change made or the reason for keeping
  the current approach.
- Push follow-up commits that are easy to review; squash only when requested or
  before merge according to maintainer preference.
- Re-run affected checks after review changes.
- Resolve a conversation only after its concern is addressed.
- Do not add unrelated changes while a pull request is under review.

## Dependency changes

- Add JavaScript dependencies with Bun, not another package manager.
- Commit both `package.json` and `bun.lock` when dependencies change.
- Keep dependency additions minimal and explain why an existing dependency or a
  small local implementation is insufficient.
- Avoid broad dependency upgrades inside an unrelated feature or fix.
- Keep Rust dependency changes scoped and commit `src-tauri/Cargo.lock` with
  `src-tauri/Cargo.toml`.
- Run frontend and Rust checks after dependency changes, even if the dependency
  appears to affect only one side.

Examples:

```sh
bun add <package>
bun add -D <package>
```

## Version, updater, and release changes

Do not change the app version as part of an ordinary pull request unless the
issue or maintainer explicitly requests it.

When a version change is required, these files must remain synchronized:

- `package.json`
- `src-tauri/Cargo.toml`
- `src-tauri/Cargo.lock`
- `src-tauri/tauri.conf.json`

Release tags use the form `v<major>.<minor>.<patch>` and must match the app
version. Creating release tags, configuring signing secrets, and publishing
releases are maintainer responsibilities.

Never commit:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- Local signing files or certificates
- Generated installer directories
- The downloaded GGUF model

Local and fork builds are expected to be unsigned. Do not weaken signature
verification or change the updater public key without an approved migration
plan.

### Local bundles and installers

Build only the local application bundle:

```sh
bun run tauri:build
```

Build every installer supported by the current platform:

```sh
bun run tauri build
```

Generated output is written below `src-tauri/target/release/bundle`. Do not
commit it.

### Maintainer release flow

1. Run the complete frontend, Rust, and applicable model-backed checks.
2. Confirm all four version declarations match the intended release.
3. Merge the verified change into `utilinlabs/preploop`.
4. Create and push a matching `v<major>.<minor>.<patch>` tag.
5. Wait for every platform installer job to complete.
6. Review the generated draft release, release notes, installers,
   `latest.json`, signatures, and `SHA256SUMS.txt`.
7. Download and test representative installers before publishing the draft.
8. Publish the release manually. CI prepares drafts but does not publish them.

Official tag builds create macOS, Windows, and Linux installers. The updater
artifacts and signatures are generated only for official version tags in the
main repository. Branches and forks build unsigned test installers and never
create an official release.

Updater signing is separate from operating-system code signing. The updater
public key is committed in `src-tauri/tauri.conf.json`; the corresponding
private key exists only in repository secrets. Losing or rotating that key
without a migration prevents installed versions from trusting future updates.

## Reporting security issues

Do not publish credentials, private user data, signing material, or a working
exploit in a public issue or pull request. Contact the maintainers privately
before disclosing a vulnerability that could place users or release integrity
at risk.

For ordinary robustness fixes without sensitive exploit details, open an issue
with reproduction steps and expected behavior.

## Troubleshooting

### The model download fails

Check your network connection and run:

```sh
bun run model:fetch
```

The downloader verifies the checksum before replacing an existing model. Do not
commit the downloaded `.gguf` file.

### The model checksum does not match

Run `bun run model:fetch` again. The setup script removes an invalid temporary
download and keeps only a verified model.

### Port 5173 is already in use

PrepLoop uses a fixed development port. Stop the process using port `5173`, then
restart `bun run tauri dev` or `bun run dev`.

### Linux cannot find a WebKit or app-indicator library

Install the packages listed in the Linux prerequisites. Package names vary by
distribution; use the official Tauri prerequisites for your distribution.

### Rust uses the wrong toolchain

From the repository root, run:

```sh
rustup show active-toolchain
rustup toolchain install 1.96.1 --component rustfmt --component clippy
```

The repository's `rust-toolchain.toml` should then select the pinned toolchain.

## Questions

If the expected behavior or contribution scope is unclear, open an issue before
investing in a large change. A small proposal with the user problem, intended
outcome, and alternatives considered is enough to start.
