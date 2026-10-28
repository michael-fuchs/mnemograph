# Using mnemograph

Conventions for filing knowledge, for agents and the owner alike. They extend the server instructions; read this page before your first write in a session.

## Where things go

| What | Where |
|---|---|
| A subject (project, technology, tool, person, concept) | Its own page, titled with the subject's common name, e.g. `Rauthy`. Other spellings become aliases. |
| A decision | Its own page of type `decision`, titled as the decision, e.g. `Use SQLite as source of truth`. |
| An observation, event or finding in progress | A note, linking every page it concerns, always including the project. |
| An area of work | A hub page of type `project` (see below). |

## Hub pages

Every project or area has one hub page. It is the entry point and contains:

- **Summary:** what it is and its current state, in a few sentences.
- **Parts:** links to its subjects, e.g. `[[mnemograph]]`, `[[Rauthy]]`.
- **Decisions:** links to its decision pages.
- **Timeline:** one line per milestone, newest first, e.g. `- 2026-10-27: v1 complete for local use, see [[mnemograph]]`.

When you create a page that belongs to a project, link it from the project's hub page in the same task.

## Decision pages

Use these sections: **Date**, **Context** (the problem and constraints), **Decision**, **Alternatives** (what was rejected and why), **Consequences**. When a decision is replaced, keep the page, add a line `Superseded by [[New decision]]` at the top, and link back from the new one.

## Notes versus pages

- Notes are the log: what happened, when, and why. They are never edited, so write each one self-contained.
- Pages hold the current, settled state. When notes about a subject have settled into knowledge, update the page so it reads correctly without the notes; the notes stay as history.
- Don't copy a page's content into notes or the other way round; link instead.

## Writing style

- Write for a reader who arrives cold: an agent in another project, or the owner months later.
- Prefer facts with dates and sources (links, issue numbers) over vague statements.
- Keep pages focused on their subject; if a section grows into its own subject, move it to a new page and link it.
- Write in English unless the subject is inherently German; search finds both languages.
