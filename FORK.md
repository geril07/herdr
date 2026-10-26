# Fork notes (`geril07/herdr`)

Personal fork of [herdrdev/herdr](https://github.com/herdrdev/herdr).
Upstream docs and `AGENTS.md` apply, except where this file says otherwise.

## Authority

- Canonical repository for this checkout is the fork (`geril07/herdr`),
  base branch `master`, maintainer `geril07`.
- Upstream `.github/MAINTAINERS`, `.github/APPROVED_CONTRIBUTORS`, and the
  upstream PR gate do not apply here. Scope is decided by the fork
  maintainer.

## Default targets

- Open pull requests into fork `master` by default, unless the human
  explicitly names another repository or branch.
- Open issues in the fork tracker by default.
- Never open pull requests or issues against upstream (`herdrdev/herdr`)
  unless explicitly asked.
- The `upstream` remote is for fetching and comparing only. Never push
  there.
- Sync from upstream through `sync/upstream-master` branches; resolve
  conflicts on the fork side.
- Fork release tags use `custom-v*`, never `v*`.

## Commits

Lowercase conventional commits with `refs #<issue>` lines for the fork
issue tracker. No closing keywords on unreleased work. Choose commit messages
without asking for approval; this overrides any upstream commit-message
approval requirement.

## Upstream API compatibility

Preserve compatibility with upstream JSON APIs and client endpoint protocols.
Keep existing methods, parameters, response fields, IDs, and their meanings
compatible. Extend APIs with optional/defaulted fields or new methods and
capabilities; clients must handle missing fork features on upstream servers.
Follow the stable client endpoint contract in `AGENTS.md`, including frozen
codecs and compatibility fixtures. Before changing an API or protocol, compare
with the upstream version integrated into the fork and verify compatibility.
If a breaking change is necessary, stop and get explicit scope approval before
implementing it.

## License

Apache License 2.0, same as upstream. See `LICENSE`.
