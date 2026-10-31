# Pages and notes carry spheres; the agent's registration sets the boundary

The owner uses one wiki for several spheres (work, private, hobbies, clubs) with different privacy needs, and many subjects span them: a page about a programming language matters at work and at home. So every page and note carries one or more spheres instead of running one wiki per sphere. Each agent registration (a pre-registered OAuth client) is bound to a fixed set of spheres, which its access tokens carry. Through it, an agent sees a page or note only if it shares at least one sphere with the registration, and writes get the registration's sphere (or, if it has several, the one the agent names). A link to a page the registration may not see shows as a stub, which reveals nothing beyond the title already written in the linking text. The owner's web login sees all spheres. A sphere may forbid that its content enters the wiki at all; the wiki then refuses that sphere.

## Considered Options

- **One wiki instance per sphere:** isolation by construction, but shared subjects would be duplicated and there would be no view across spheres.
- **Exactly one sphere per page,** with a common sphere for neutral knowledge: simpler rules, but pages that belong to two spheres would have to be split or put where everyone sees them.
- **The agent names the sphere in each request:** convenient, but no boundary.

## Consequences

Every query that returns pages, notes, links, search hits, graphs or statistics must filter by the registration's spheres, so this filtering lives in one place in the core and is tested for each feature. An agent installation that works in several sphere sets (work and private repositories on one machine) needs one registration per set. Existing pages and notes need a sphere when the feature arrives.
