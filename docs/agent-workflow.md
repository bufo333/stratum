# Agent Workflow

This repository uses one workflow across agent tools. Tools do not share session
state; they share this process, `AGENTS.md`, and immutable plan artifacts.

## Roles

| Role | Lifetime | Authority |
| --- | --- | --- |
| `coordinator` | One interactive session | Dispatches and verifies handoffs; never plans, edits, or changes Git state. |
| `bootstrapper` | One governance draft, freeze, or commit operation | Creates the initial project-specific governance baseline only. |
| `planner` | One draft or freeze operation | Inspects and writes only local plan artifacts. |
| `branch-bootstrap` | One approved branch | Creates an approved branch from the declared base; never edits or commits. |
| `implementer` | One approved task | Implements or corrects one approved plan and verifies it. |
| `reviewer` | One committed revision | Independently reviews the exact revision; never writes. |
| `integrator` | One accepted revision | Fast-forwards an accepted branch into local main; never edits. |

## Governance Bootstrap

When `docs/engineering-contract.md` is absent, normal delivery is unavailable.
The coordinator asks the questionnaire in `docs/governance-bootstrap.md`, then
dispatches a fresh bootstrapper with the recorded answers.

The bootstrapper drafts only the project-specific documents named in that guide.
The coordinator presents a SHA-256 for every proposed file. The user approves the
exact set of hashes before a fresh bootstrapper freezes or commits anything.

After explicit approval, a bootstrapper commits only the approved governance
baseline locally. It must not create a branch, alter remotes, or make unrelated
changes. If the repository has no Git history, it may create the initial local
commit. The user performs all remote operations.

## Plan Artifacts

Planning state is local and ignored under `.ai/plans/`.

- `.ai/plans/draft.md` is mutable. A fresh planner may overwrite it after a current
  repository inspection.
- `.ai/plans/approved/<branch-slug>-<base-short-sha>-<plan-short-sha>.md` is the
  immutable, byte-identical snapshot approved by the user.

The coordinator hashes a draft before presenting it. Approval applies only to that
SHA-256. The planner freezes the draft only after verifying that hash. A mutable
draft, pasted conversation text, or summary is never an implementation handoff.

## Delivery

1. A fresh planner inspects the repository and writes one complete draft.
2. The coordinator presents its SHA-256. The user approves or requests a new draft.
3. A fresh planner freezes the verified draft under `.ai/plans/approved/`.
4. A fresh branch-bootstrap agent creates the approved branch from the exact base.
   **Pre-condition:** Verify there is no open implementation branch (i.e.,
   `main` is the only local branch, or all feature branches have been merged
   into `main`). If an open feature branch exists, stop and report — do not
   create a new branch.
5. A fresh implementer verifies the snapshot, branch, and base before editing,
   then runs the project-specific gate and commits.
6. A fresh reviewer verifies the same snapshot and reviews the exact commit.
7. Confirmed in-scope findings go to an implementer in correction mode. A material
   scope, behavior, architecture, contract, or governance change requires a new
   approved artifact.
8. A fresh integrator verifies the accepted commit, fast-forwards local `main`,
   and deletes the branch.
9. Agents stop. The user owns remote operations.

## Boundaries

Work starts only from a user-approved plan after governance bootstrap. One
implementation branch may be in flight. Branch creation, commits, fast-forward
merges, and branch deletion require an explicit permission prompt in the active
tool.

Technical decisions belong to the fresh planner. The coordinator asks the user
only for unresolved product or architecture policy, approval of exact hashes,
material plan revisions, and Git permission prompts.
