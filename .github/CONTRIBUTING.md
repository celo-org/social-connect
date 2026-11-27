# Contributing

Thank you for considering making a contribution to SocialConnect and the Celo community!
Everyone is encouraged to contribute, even the smallest fixes are welcome.

If you'd like to contribute to Celo, please fork, fix, [write a changeset](../RELEASE.md), commit, and send a pull request for the maintainers to review.

If you wish to submit more complex changes, please sync with a core developer first.
This will help ensure those changes are in line with the general philosophy of the project
and enable you to get some early feedback.

See the [community contribution guide](https://docs.celo.org/community/contributing) for details on how to participate.

## Directory Structure

<pre>
├── <a href="../docs">docs</a>: Documentation on how SocialConnect works and how to use it
├── <a href="../apps">apps</a>: Contains the monitor, with health checks and load tests for ODIS
│   ├── <a href="../apps/monitor">monitor</a>: Monitoring service that sends health checks to deployed ODIS instances. Also contains code for load testing
├── <a href="../packages">packages</a>: Contains all published SocialConnect components
│   ├── <a href="../packages/identity">identity</a>: SDK for using SocialConnect
├── <a href="../e2e">e2e</a>: End-to-end tests of the ODIS signer and combiner APIs against running services
├── <a href="../rust">rust</a>: The ODIS signer and combiner
├── <a href="../scripts">scripts</a>: Misc. deployment and release scripts
</pre>

## Dev Setup

### Pre-requisites

* [Git](https://git-scm.com/downloads)
* [NodeJS](https://nodejs.org/en/download/)
* [Node Version Manager](https://github.com/nvm-sh/nvm)
* [rustup](https://rustup.rs/), which installs the Rust version pinned in `rust-toolchain.toml`
* [just](https://github.com/casey/just), which runs the tasks in the `justfile`
* [cargo-nextest](https://nexte.st/), the test runner that `just test` uses
* [Docker](https://docs.docker.com/get-docker/), optional: without it, the Postgres and differential tests skip

### Setup

Clone the repository and open it:

```bash
git clone git@github.com:celo-org/social-connect.git
cd social-connect
```

#### Install the Correct Version of NodeJS

Install the correct node version with [nvm](https://github.com/nvm-sh/nvm)

```bash
nvm use
```

#### Install node modules and build with yarn

First, run 

```bash
corepack enable
```

This will setup the required `yarn` version

To install dependencies, run

```bash
yarn
```

To build all components (in both `/apps` and `/packages`) run

```bash
yarn build
```

#### Running tests

Our testing suite uses unit, integration and e2e tests. Integration tests spin up and run against local ODIS instances with in-memory DBs, while e2e tests run against actual deployed ODIS instances on Celo Sepolia or mainnet.

To run the tests in `/packages` run

```bash
yarn test
```

To run the unit and integration tests of the ODIS signer and combiner (in `/rust`) run

```bash
just test
```


To run ODIS e2e tests, navigate to the `e2e` directory, which holds the suites for both the Combiner and the Signer. You can specify the environment and the service to run against as in the following example (see `e2e/package.json` for all available test commands).

```bash
cd e2e
DEPLOYED_SIGNER_SERVICE_VERSION=<version it reports> yarn test:signer:celo-sepolia:0
```

Set `DEPLOYED_SIGNER_SERVICE_VERSION`, or `DEPLOYED_COMBINER_SERVICE_VERSION` for the combiner suite, to the version the service under test reports at `/status`; the suite compares it with the version the service reports in its answers.

To check a mainnet signer's keys, run only the "[Signer configuration test]" case, which checks one signature against the mainnet polynomial:

```bash
CONTEXT_NAME=mainnet ODIS_SIGNER_SERVICE_URL=<signer URL> DEPLOYED_SIGNER_SERVICE_VERSION=<version it reports> yarn test:signer -t '\[Signer configuration test\]'
```

For load tests, checkout [apps/monitor](../apps/monitor/README.md)

#### Commit messages

Subjects follow [Scoped Commits](https://scopedcommits.com): `<scope>: <description>`, for example `monitor: set CHAIN_ID for the mainnet deploy`. There is no type prefix such as `feat` or `fix`.

* The scope names the component the commit changes. An app, package or workspace takes its directory name (`monitor`, `identity`, `e2e`). A Rust crate takes its directory name under `rust/` plus `-rs` (`signer-rs`, `combiner-rs`), which keeps it apart from the TS app of the same name on `release/odis-ts-4.x`. Changes outside the components use `ci`, `deps` or `docs`.
* A commit that changes two components names both, separated by a comma. One that changes the whole repo uses `treewide`.
* Merges and reverts keep Git's default subject.
* The description is imperative, starts in lower case and has no full stop. It does not repeat the scope: `signer-rs: add JSON log output`, not `signer-rs: add JSON log output to the Rust signer`.

#### PRs and Releases

See [Release.md](../RELEASE.md)
