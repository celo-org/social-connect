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

changesets has 2 strategies for pre release versions.

The first is to enter `pre` mode on changesets. [docs here](https://github.com/changesets/changesets/blob/main/docs/prereleases.md)

```
yarn changeset pre enter beta
yarn changeset version
git add .
git commit -m "Enter prerelease mode and version packages"
yarn changeset publish
git push --follow-tags
```

The other is to append --snapshot. which is great for daily releases.

```
yarn changeset version --snapshot canary

yarn changeset publish --no-git-tag --snapshot

```

<https://github.com/changesets/changesets/blob/main/docs/snapshot-releases.md>

## Package Versioning

Based on semantic versioning best practices [semver.org](semver.org)

Given a version number MAJOR.MINOR.PATCH, increment the:

* MAJOR version when you make incompatible API changes
* MINOR version when you add functionality in a backward compatible manner
* PATCH version when you make backward compatible bug fixes

Additional labels for pre-release and build metadata are available as extensions to the MAJOR.MINOR.PATCH format.
