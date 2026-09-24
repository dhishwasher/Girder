# Addendum 2 to labeling-rubric.md: a structural-typing rule for class receivers

Written AFTER the 105 sites were labeled but BEFORE any of them was
rescored, per a further review's request. Recorded honestly as such,
not backdated into the rubric itself.

## Case 1 addition: private/protected members close the structural-substitution gap

Case 1's rule for `this.m()`/instance receivers assumed that "no override
anywhere in the snapshot" is sufficient for Must. A further review pointed
out a gap specific to TypeScript: classes are compared **structurally**
unless they declare at least one `private`, `protected`, or `#private`
member -- without one, ANY object with the same public shape is
type-assignable to a class-typed parameter, even if it never went through
that class's own constructor. A class-typed receiver coming from an
external, caller-supplied parameter could therefore, in principle, be
satisfied by an unrelated structurally-compatible object, not just a
genuine instance or subclass instance.

**Rule, added to case 1**: before labeling Must for a class-typed
receiver, check whether the class declares any `private`/`protected`/
`#private` member. If it does, the class is effectively nominal (no
unrelated object can satisfy it), and the existing "no subclass override"
check is sufficient on its own. If it does NOT, an additional check is
needed: is the receiver's actual value CALLER-SUPPLIED (an external
parameter, where an adversarial or merely different caller could pass a
structurally-compatible-but-unrelated object) or INTERNALLY-CONTROLLED
(e.g. returned by one of the class's own lookup/factory methods querying
its own private state, where the runtime value is provably always a
genuine instance regardless of the type's structural permissiveness)? Must
requires the latter, or a private/protected member closing the gap
outright; a purely-structural class type on a genuinely externally-
supplied parameter is Unknown.

Applied retroactively to the three sites this matters for in the
committed labeling (`675528d`, corrected in the same round as this
addendum): `TestState` (sites 54, 55) has four `private` members,
closing the gap outright. `ScriptInfo` (site 65) has none, but its
receiver traces to an internal `ProjectService` lookup method, not a
caller-supplied parameter -- Must holds under the second branch of this
rule, not the first.
