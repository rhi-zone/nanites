# Maybe Rules

Rules observed in ecosystem repos that might be worth adding to the scaffolding template,
but deferred as potentially too abstract or noisy for a general baseline.

## "No tunnel vision"
*Source: existence/CLAUDE.md*

> The most dangerous moments are the ones that feel small and mechanical — a quick conversion,
> a direct translation, a minor change. These are exactly when the surrounding context stops
> being examined. Every change, however routine, is an opportunity to ask whether the thing
> being changed still makes sense.

Concern: abstract, hard to act on without a concrete trigger.

## "Before proposing any fix, ask 'what IS this?'"
*Source: existence/CLAUDE.md*

> When investigating a problem, the first question is not "where should this go?" or "what
> should this value be?" — it's "what is this concept, in full generality, and what system
> owns it?" The fix follows from that answer. Skipping straight to a patch embeds the wrong
> frame.

Concern: good philosophy but abstract. Works better as a project-specific rule once a
concrete failure mode has been observed.

## "Implementation priority ≠ documentation priority"
*Source: existence/CLAUDE.md*

> Deferred items need documentation *more* than urgent ones — urgent things get built and the
> code becomes the record. A deferred insight exists only in docs or not at all. "Not urgent
> to implement" is the strongest reason to write something down immediately, not a reason to
> keep it in chat.

Concern: mostly covered by "note things down immediately — no deferral" + "TODO.md before
implementation". This adds a useful angle (deferred items need docs more) but may be noise
as a separate bullet.
