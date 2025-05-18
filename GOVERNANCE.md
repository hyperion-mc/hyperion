# Hyperion Governance

This document outlines the simple governance structure for Hyperion, a high-performance Minecraft game engine.

## Project Roles

### 1. Project Lead
- Project founder (@andrewgazelka) and other designated leads
- Has final authority on project direction
- Controls access to critical resources (repositories, hosting, domains)
- Resolves disputes when consensus cannot be reached

### 2. Core Contributors
- Demonstrated consistent high-quality contributions
- Push access to all repositories
- Can approve significant changes
- Help shape project direction

### 3. Contributors
- Anyone who has contributed to the project
- No special permissions, but acknowledged for their work
- May be nominated for Core Contributor status

## Decision Making

### Code Changes
- Minor changes (bug fixes, small improvements) need one Core Contributor approval
- Major changes (API, architecture) need discussion and two Core approvals
- Core Contributors can merge their own minor changes for areas they maintain
- Always prioritize performance, scalability, and stability

### Project Direction
- Major decisions are discussed openly in GitHub issues or Discord
- Core Contributors aim for consensus on important matters
- If consensus cannot be reached, Project Lead makes the final decision
- Technical merit and alignment with project goals are primary criteria

### Governance Changes
Changes to this document require discussion and consensus among Core Contributors and Leads.
Proposed changes must be submitted in the form of a PR. The discussion should occur in the PR reviews and comments.

Minor changes like typos (1-2 characters changed, fixing URLs) or formatting (reordering sections of text verbatim,
changing whitespace, etc.) do not require this process, but should still follow the usual PR review process.
Minor changes require 1 Lead approval.

A governance change requires all Core Contributors and Leads (voting members) to approve the change by submitting
an approving PR review, or to abstain. Upon receiving all approvals, the change will be merged.

All voting members must be notified both in discord and via github. Requesting a review from a voting members
is sufficient to notify them on github. Voting members will have 2 weeks to respond to the governance change,
after which they will automatically be considered to have abstained. If a voting member has responded to the
governance change raising concerns, they *cannot* be automatically considered to have abstained.

## Becoming a Core Contributor

1. Make consistent contributions to the project
2. Get nominated by an existing Core Contributor
3. A plurality of Core Contributors votes to approve (must all be @'d in Discord and have four days to respond)

## Code Review

We value quick iteration while maintaining technical excellence:

- Focus reviews on correctness, performance, and maintainability
- Include performance benchmarks for changes to critical paths
- Ensure adequate test coverage for new features
- Use GitHub Pull Requests for all changes
- Prefer small, focused changes over large changes
- Document design decisions that affect scalability

## Communication

- GitHub Issues: Technical discussions and bug reports
- Discord: Community support and development coordination
- Technical discussions should be direct and solution-oriented
- Focus on the work, not the person

## Conflict Resolution

1. Technical disagreements should first be discussed on relevant issues/PRs
2. If unresolved, Core Contributors discuss and seek consensus
3. Project Lead makes final decision when consensus cannot be reached

## Moderation

- Follow the Hyperion Code of Conduct
- Violations can be reported to project leads via Discord DM or email
- Project Lead will review reports and determine appropriate action
