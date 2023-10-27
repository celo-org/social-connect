# Release Process

This repo uses changesets to determine what packages need a version bump.

Each PR MUST be accompanied by a changeset unless it has zero affect on package consumers (ie changing github action workflows).

To create a changeset run `changeset add` (or  `yarn cs`)

This will bring up an interactive console which asks which packages are affect and if they require minor or major update.

## Releasing

This repo is setup to automatically version and release packages

Each time a changeset is merged into main a "Version Packages" PR will automatically be opened showing the next versions for all packages and convert changeset files into changelog.md. Merging this PR will

* publish packages to NPM
* create GH release notes

## For pre releasing

For Detailed Steps read scripts/beta-mode.sh

1. Run `yarn beta-enter`
This will enter into the pre mode of changesets and create a prerelease/beta branch and push it up to origin(github)

Any time a commit is pushed to prerelease/** github will go and open a specially Version Packages (Beta) PR. You can merge this and packages will be published as specified in the branch (should be beta)

2. If you need to release another beta make a changeset and commit it up.

3. When done run `yarn beta-exit`
This will exit changeset pre mode. Push up.

4. Now you can Open a Pr with your prerelease/? branch against main.

## Package Versioning

Based on semantic versioning best practices [semver.org](semver.org)

Given a version number MAJOR.MINOR.PATCH, increment the:

* MAJOR version when you make incompatible API changes
* MINOR version when you add functionality in a backward compatible manner
* PATCH version when you make backward compatible bug fixes

Additional labels for pre-release and build metadata are available as extensions to the MAJOR.MINOR.PATCH format.
