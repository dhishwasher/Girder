# Contributing to Girder

## Licensing of contributions

Girder is source-available under the
[Business Source License 1.1](./LICENSE). Unless you state otherwise in
writing, any contribution you intentionally submit for inclusion in this
repository is offered under that same license, including its Additional Use
Grant, Change Date, and Change License, with no additional terms or
conditions.

Please sign off your commits under the
[Developer Certificate of Origin](https://developercertificate.org/) (DCO):

```bash
git commit -s -m "your message"
```

which appends:

```
Signed-off-by: Your Name <your.email@example.com>
```

That line is you asserting you wrote the contribution, or otherwise have the
right to submit it under the licenses above. It is a statement about
provenance, not an assignment of your copyright — you keep that.

### Why this is stated up front

Clear contribution terms ensure that contributors knowingly grant the rights stated above. They avoid ambiguity for contributors and downstream users while preserving contributor copyright.

Stating the terms now costs a contributor one command-line flag. Discovering
they were never stated, after a hundred contributors, is not fixable at any
price. Nothing here asks you to assign copyright or sign a CLA.

## Before you open a pull request

The binding constraint on this repository: **the default build must compile
and the whole test suite must pass.**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets   # CI runs this with -D warnings
cargo test --workspace
```

CI additionally type-checks the `gui` feature, compiles the `live-providers`
feature, runs the measurement-harness unit tests, and runs the headless demo.
See [`.github/workflows/ci.yml`](./.github/workflows/ci.yml).

Project conventions — the semantic graph as source of truth, path-derived node
ids, project-wide call resolution, not holding the graph mutex across an
`await`, and keeping the offline `MockProvider` as the terminal fallback — are
documented in [`CLAUDE.md`](./CLAUDE.md). Read that before changing anything
structural.

## Measurements and claims

This repository holds itself to an unusual standard, and it is the most
valuable thing about it: performance and accuracy claims are backed by a
precommitted policy file, a recorded observation, and a harness that
regenerates the number. Failures are recorded as failures rather than
retuned away — see [`docs/authoring-cost.md`](./docs/authoring-cost.md) and
[`docs/context-vs-read-cost.md`](./docs/context-vs-read-cost.md) for what that
looks like in practice.

If you add or change a claim about cost, speed, or accuracy:

- Write the policy (inputs, baseline, metric, threshold) **before** measuring.
- Check in the raw observation, whatever it says.
- Provide a way to regenerate it.
- If it fails its threshold, record that it failed.

A plausible number with no method behind it is worse than no number, because
someone downstream will repeat it. One such claim already shipped here and had
to be removed; `docs/context-vs-read-cost.md` documents that specific
incident and what replaced it.
