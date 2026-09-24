# Stage 3 TypeScript audit methodology, frozen before any site is read

Mirrors Python's own frozen-before-reading-any-site discipline
(`docs/observations/stage3-python-audit/methodology.md`, itself mirroring
Rust's). Every decision below was made and committed before any of the 105
selected sites in `audit-sites.json` was opened.

## 1. Corpus

Three sha256-verified TypeScript repositories, pinned in
`docs/stage3-typescript-corpus.json` -- **deliberately a separate manifest
from `docs/core-representative-corpus.json`**, not an extension of it. The
first attempt added these three repos directly to the shared file
(`fbd39f8`) and broke that file's own already-gated test suite:
`tools/core_representative_benchmark.py`'s `validate_manifest()`
hard-codes "exactly six repositories", a `rust`/`python`-only language
allowlist, a single-extension-per-language assumption
(`expected_extension = ".rs" if language == "rust" else ".py"`, which
cannot express TypeScript's `[".ts", ".tsx"]`), and requires every entry to
carry a nonempty `semantic_cases` list before it will validate at all.
Reverted (`00ba4b1`) and re-pinned in a dedicated manifest instead
(`e7270bd`). Grepping for readers of the `semantic_cases` field (confirmed
no Stage 3 site-selector tool reads it) was checked BEFORE the first
attempt but was not sufficient evidence of safety -- it shows nothing reads
the field from the OUTSIDE, not that the shared file's own internal
validation wouldn't reject the addition. The lesson, recorded so it isn't
relearned: "who reads a field" and "does the file's own validator accept
this shape" are two different questions, and both need checking before
extending a shared, gated manifest.

- **`typescript-6.0.3`**: the TypeScript compiler's own source
  (`src/` only -- the full upstream repository tarball has 84,334 members,
  mostly its own conformance-test baseline corpus, and was extracted with
  a path-filtered subset extraction rather than the whole archive; see
  `tools/dispatch_audit_site_selector_typescript.py`'s
  `extract_archive_subset`). This is the "TypeScript compiler snapshot"
  the roadmap's Stage 3 order names explicitly, parallel to Go's planned
  standard-library pin. Memory feasibility was checked directly before
  choosing this version, not assumed: `girder analyze` against the full
  709-file source tree peaked at 473MB RSS / 4:36 wall time on this
  2.7GB-RAM VM. Version chosen deliberately: TypeScript 7.x is the native
  Go rewrite ("tsgo") and ships no `.ts` compiler source at all -- checked
  directly (`tar tzf` on the v6.0.3 tag's archive, confirmed 77 files
  under `src/compiler/` including `checker.ts`) before downloading,
  precisely because a version shipping no real TypeScript source would
  defeat the entire audit.
- **`zod-3.23.8`**: reused the exact tarball/sha256 already pinned in the
  separate, pre-existing `docs/typescript-support-policy.json` (a
  TypeScript-extraction smoke-test corpus, unrelated to this audit and
  confirmed unaffected by anything in this round), rather than fetching a
  newer version, since it was already verified. Gives a
  validation-library dispatch profile roughly parallel to Python's
  pydantic.
- **`date-fns-4.1.0`**: a real-world utility library (1,529 small
  per-function files), parallel in spirit to requests/petgraph's
  "ordinary application code" role in the other two languages' audits.

## 2. Prerequisite check: does Girder even expose per-call-site TypeScript evidence?

Confirmed directly on an existing dispatch-corpus TypeScript fixture
(`fixtures/dispatch-corpus/typescript/direct-same-file`), not assumed:
`girder analyze`/`inspect --json` exposes the same `call_evidence_v1`
structure for TypeScript as for Rust/Python -- byte-precise `site` spans,
`class`, `targets`, `reason`, per call, on both function and module nodes.

## 3. Site enumeration is independent of Girder's own parser

Same discipline as Rust and Python: a call site Girder's tree-sitter
grammar fails to see would never enter a Girder-derived sample, so sites
are found with a plain, hand-written character scanner over the three
pinned repositories' `.ts` files
(`tools/dispatch_audit_site_selector_typescript.py`), not `girder query`.

## 4. Masking: no stdlib tokenizer exists for TypeScript/JavaScript

Python's audit reused the stdlib `tokenize` module for masking. There is
no equivalent in Python's standard library for TypeScript/JavaScript, and
this repository's Python tooling has no third-party dependency
infrastructure at all (no `requirements.txt`/`pyproject.toml`, confirmed
by inspection; `pip install` for `tree-sitter`/`tree-sitter-typescript`
was attempted and blocked by this Debian install's externally-managed-
environment protection before this was even a real option, not just a
theoretical one). `mask_ts_source` is therefore a hand-written,
single-pass character scanner, independent of Girder's own parser the same
way the regex layer above it is.

It masks line comments (`//`), block comments (`/* */`), single/double-
quoted string literal text, template literal (backtick) text, and regex
literals -- but deliberately does **not** mask `${...}` template
interpolation expressions, since those are real code that can contain a
real call site (`` `Hello ${getName()}` ``). Nested interpolation
(`` `${`${real(1)}`}` ``) and an object literal inside an interpolation
(`` `${JSON.stringify({a: 1})}` ``, where the literal's own `{`/`}` must
not be mistaken for the interpolation's closing brace) are both handled
via an explicit frame stack, not just a flat state machine -- both are
covered by dedicated unit tests, not just reasoned about.

Regex-literal masking requires disambiguating a leading `/` from the
division operator, a well-known JavaScript lexing ambiguity. Resolved with
the same context-based heuristic real JS tokenizers use: a `/` starts a
regex unless the last significant token was an identifier/number/`)`/`]`
that doesn't belong to a small set of "still expects an operand" keywords
(`return`, `typeof`, `new`, `case`, ... -- the full set is
`REGEX_CONTEXT_KEYWORDS` in the tool). This is an approximation, not a
full JS parser, and is disclosed as such -- checked against representative
synthetic cases (division after an identifier, after a closing paren,
regex after `return`, a character class containing an unescaped-looking
`/`) in the tool's own test suite, and empirically against the real
corpus below, not assumed correct by construction.

**Empirical masking effect, checked against all three real pinned
repositories, not just synthetic snippets**: 150,719 raw (unmasked)
call-shaped line matches drop to 132,578 after masking -- a **12%
reduction** (typescript-6.0.3: 119,952 &rarr; 103,672; zod-3.23.8: 10,459
&rarr; 9,806; date-fns-4.1.0: 20,308 &rarr; 19,100). Smaller than Python's
33% (click/pydantic/requests' docstrings are unusually code-example-heavy;
this corpus's comments and test-fixture strings are less so), but real and
non-trivial -- masking is required here too, not a skippable step the way
it was for Rust's own audit.

Unlike Python's `tokenize`-based masker, `mask_ts_source` has no failure
mode analogous to `tokenize.TokenError`: an unterminated string, template,
regex, or comment at end-of-file is simply masked to the end of the file
rather than raising, so there is no `tokenize_failure_count`-equivalent
field to report (documented in the function's own docstring, not a gap
silently left unexplained).

## 5. Shapes: no operator-overloading analogue; optional chaining is new

Chosen for TypeScript's own dispatch-ambiguity landscape, not copied
mechanically from Python or Rust's shape lists:

- `decorator` -- `@Something`/`@Something(...)` on classes, methods,
  properties, or parameters. Same spirit as Python's `@decorator`
  and Rust's attribute-macro handling.
- `dynamic_dispatch` -- bracket-notation calls (`obj[prop]()`),
  `.call`/`.apply`/`.bind`, `Reflect.apply`/`Reflect.construct`, and
  `new Function(...)`. The TypeScript/JavaScript analogue of Python's
  `getattr`/`setattr`/`callable` shape.
- `optional_chaining_call` -- `obj?.method()` / `fn?.()`. **A genuinely
  TypeScript/JavaScript-specific dispatch uncertainty neither Rust nor
  Python has**: the call may or may not happen at all, depending on
  whether the receiver is nullish, which is a distinct kind of
  uncertainty from "which method gets dispatched to."
- `new_expression` -- `new Foo(...)`, including generic type arguments
  (`new Map<string, number>()`). Parallel to Python's direct-construction
  case (`x = Foo(); x.m()`), relevant for the same
  `__new__`/metaclass-equivalent construction-time dispatch questions the
  Python rubric's case 4 covers (TypeScript's own analogues: a class with
  a custom `static` factory pattern, or a `Proxy`-wrapped constructor).
- `qualified_attribute_call` -- `a.b.c(...)`, three or more dotted
  segments before the call, same spirit as Python's shape of the same
  name.
- `method_call` -- `.name(...)`, the single most common dispatch-
  ambiguous shape given TypeScript's structural typing and interface-based
  polymorphism.
- `plain_call` -- bare identifier call, the generic fallback.

**No `operator_dunder`/`operator_usage` analogue**: TypeScript, like
JavaScript, has no operator overloading at all -- `+`/`-`/`==`/etc. always
mean the same built-in thing regardless of operand type (with the single,
narrow exception of `valueOf`/`Symbol.toPrimitive` coercion, which is not
call-site dispatch ambiguity in the sense Rust's `operator_usage`/Python's
`operator_dunder` shapes exist to capture). This category is simply absent
from the TypeScript shape list, not stubbed out with an empty pattern.

`KEYWORDS_NOT_CALLS` includes TypeScript-specific keywords Python/Rust
don't have (`interface`, `type`, `enum`, `namespace`, `declare`,
`readonly`, `abstract`, `implements`, `extends`, `as`, `satisfies` is not
yet needed since it doesn't precede a paren the way other keywords can).
Checked directly, not assumed complete: a function/method/class
*declaration*'s own `name(...)` is still picked up as `plain_call`-shaped
by this selector (`function foo(x) {` classifies as `plain_call`, same as
Python's `def target():` does) -- this is deliberate, matching Python's
own established precedent (`labeling-rubric.md` case 9): excluding a
declaration site is a **ground-truth labeling** decision
(`not_a_call_site`), not a selector-level exclusion. The selector stays
deliberately crude and over-inclusive; precision belongs to the labeling
step, not the sampler.

## 6. File set and stratification

All `.ts` files under each pinned repository's root are included (no
directory exclusions), matching Rust and Python's own precedent. `.tsx`
files are declared in `zod-3.23.8`/`date-fns-4.1.0`'s `sources.extensions`
but the selector itself only globs `*.ts` (`rglob("*.ts")`) -- **`.tsx`
files are out of scope for site selection in this round**, disclosed here
rather than silently included or silently promised. Neither zod nor
date-fns's actual runtime source uses JSX (both are libraries, not UI
code); this is expected to have zero practical effect on the sample, but
is recorded as an explicit scope decision, not verified empty by
inspection this round.

Stratified by shape only (seven shapes: `decorator`, `dynamic_dispatch`,
`optional_chaining_call`, `new_expression`, `qualified_attribute_call`,
`method_call`, `plain_call`), the same deterministic per-shape shuffle
under a fixed seed (`20260924`) Rust and Python's selectors use, capped
per shape and topped up round-robin from leftover shapes. Not also
stratified by repository: `typescript-6.0.3`'s pool (103,672 masked
matches) dwarfs zod's (9,806) and date-fns's (19,100), so the compiler
dominates the selected 105 (`package_counts_selected`:
`typescript-6.0.3` 78, `date-fns-4.1.0` 21, `zod-3.23.8` 6) -- disclosed
here rather than corrected by adding a second stratification dimension,
for the same reason Python's methodology gave: keeping the frozen
algorithm identical to precedent rather than introducing new, untested
sampling logic at freeze time.

## 7. Ground-truth labeling rubric

Not yet written. Per the frozen sequence (this document, then repo pins
[done, `e7270bd`], then the site selector plus its tests [done, same
commit], then at least 100 selected sites [done, see \S8], then the
rubric, written before reading any site's `true_class`) -- the rubric is
the next piece, committed separately before any site is labeled.

## 8. "Zero classification errors" definition, unchanged

Per the frozen Stage 3 policy (`docs/roadmap.md`), identical to Rust and
Python: an error is an **unsound** audit cell -- `overclaim` (a false
Must, or a false May with no viable candidate set) or `unsafe_exclusion`
(a reachable call site excluded from the graph's reasoning entirely). An
honestly-labeled `unknown` result, even where the true answer is
`must`/`may`, is a conservative miss, not an error. Not renegotiated for
TypeScript.

## 9. What is NOT yet done (recorded so a future session doesn't assume otherwise)

- 105 sites were selected (`tools/dispatch_audit_site_selector_typescript.py
  --output docs/observations/stage3-typescript-audit/audit-sites.json`,
  seed `20260924`) but have not been read or labeled -- no
  `true_class`/rationale fields exist yet.
- No TypeScript-specific scorer (`dispatch_audit_scorer_typescript.py`,
  mirroring the Python scorer's own necessary rewrite -- Rust's scorer's
  `site_byte_offset` imports Rust's own `SHAPE_PATTERNS` by name and would
  `KeyError` on the first TypeScript-shaped site) has been written yet.
- No before-observation measurement has been run.
- No gate-profile step (mirroring Rust's `dispatch_audit_gate_profile.py`
  and Python's `dispatch_audit_gate_profile_python.py`) has been run --
  `transformed_scope` (any decorator anywhere in a file) still gates
  TypeScript's whole-file Must computation exactly as it did for Python
  before that language's own resolver round, deliberately left untouched
  through Python's Stage 3 work to respect language order, and is the
  obvious first gate-profile suspect here too.
- The dispatch corpus's own pre-existing `typescript-structural-object-
  literal` failed cell (origin symbol unresolved, confirmed unchanged
  since `after-operator-claim-fix`, not new) has not been investigated or
  fixed.
