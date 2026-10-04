# Roles and skills

Direct keeps immutable revisions of agent role contracts and selected
project-scoped skill bundles (DIR-74). The Development Operating System stays
the maintained source of methods: a role revision *pins* DOS documents through
Theoria (fingerprint and, only when recorded, playbook version) and never
edits them. Role and skill text is task context; no Direct permission reads it.

## Records

- **Skill package** (`register_skill_package`): one bundle revision. Files are
  stored with SHA-256 hashes and a bundle hash. Paths must be plain relative
  segments; the bundle needs `SKILL.md` whose frontmatter declares the same
  `name` and a `description`. `origin` is `local` or `upstream`; an upstream
  skill needs `owner/name` and an immutable commit SHA (no branch or `latest`).
  An unchanged bundle reuses its revision. The owner retires revisions
  (`retire_skill_package`).
- **Agent role** (`register_agent_role`): a draft revision with
  responsibilities, inputs, outputs, exact skill revisions, runtime
  compatibility, guidance pins (`mandatory` per pin) and owner direction.
- **Publication**: the files one project-scoped publication introduced, plus
  fresh-session activation evidence and its rollback.

Agents and the owner register drafts, publications and evidence. Only the owner
activates (`activate_agent_role`) or retires (`retire_agent_role`) a role
revision. Activation is refused when the revision has no owner direction, uses
a retired skill, or a mandatory pinned DOS document is unavailable. A newer
activated revision supersedes the previous active one. Drift (the DOS source
changed after pinning) is shown and never rewrites a pin; adopt changes by
registering a new revision. Unknown playbook versions stay Unknown.

## CLI

```powershell
# Register the selected skill bundle from a directory.
direct --actor <agent> roles register-skill --dir agents\skills\direct-issue-health `
  --name direct-issue-health --description "..." --trigger "..." `
  --license "..." --request-id <id>

# Preview a publication (writes nothing), then apply it.
direct --actor <agent> roles publish --role-id <role revision id> --dest <project dir>
direct --actor <agent> roles publish --role-id <id> --dest <project dir> --apply --request-id <id>

# Record a fresh harness session's output, then roll back when done.
direct --actor <agent> roles evidence --publication-id <id> --session-id <id> `
  --model <model> --marker "<expected output marker>" --output-file out.json --request-id <id>
direct --actor <agent> roles rollback --publication-id <id> --request-id <id>
```

Publishing supports Claude Code's project layout: `.claude/skills/<skill>/…`
and `.claude/agents/<role>.md`. Other harnesses show as unverified until a
fresh session has been recorded for them. The CLI refuses:

- destinations that are missing, a filesystem root, the home directory or any
  ancestor of it, or inside `~/.claude`, `~/.codex`, `~/.agents` or `~/.config`;
- paths that would escape the destination, including through links;
- any planned file that already exists with different content (nothing is
  written; identical files are left alone and not recorded as introduced).

Rollback deletes only introduced files whose content still matches the recorded
hash, keeps and reports modified ones, and removes only directories it left
empty.

The repository keeps the reviewed sources: `agents/skills/direct-issue-health`
(the selected skill) and `agents/roles/product-planner.json` (the role request).
