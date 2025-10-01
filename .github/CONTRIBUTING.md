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
├── <a href="./docs">docs</a>: Documentation on how SocialConnect works and how to use it
├── <a href="./kubernetes-deployments">kubernetes-deployments</a>: YAML config files and instructions for ODIS deployment
├── <a href="./apps">apps</a>: Contains deployed componentes of Oblivious Decentralized Identifier Service (ODIS) for SocialConnect
│   ├── <a href="./apps/combiner">combiner</a>: Orchestrates distributed BLS threshold signing with the set of ODIS signers - requests and combines partial signatures
│   ├── <a href="./apps/monitor">monitor</a>: Monitoriing service that sends health checks to deployed ODIS instances. Also contains code for load testing
│   ├── <a href="./apps/signer">signer</a>: Generates unique partial signatures for blinded messages
├── <a href="./packages">packages</a>: Contains all published SocialConnect components
│   ├── <a href="./packages/common">common</a>: Contains common logic for ODIS, including API schemas
│   ├── <a href="./packages/encrypted-backup">encrypted-backup</a>: PEAR account recovery SDK, powered by ODIS
│   ├── <a href="./packages/identity">identity</a>: SDK for using SocialConnect
│   ├── <a href="./packages/odis-identifiers">odis-identifiers</a>: Contains identifier prefixes and hashing functions for ODIS
├── <a href="./scripts">scripts</a>: Misc. deployment and release scripts
</pre>

## Dev Setup

### Pre-requisites

* [Git](https://git-scm.com/downloads)
* [NodeJS](https://nodejs.org/en/download/)
* [Node Version Manager](https://github.com/nvm-sh/nvm)

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

First, run:

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

Our testing suite uses unit, integration and e2e tests. Integration tests spin up and run against local ODIS instances with in-memory DBs, while e2e tests run against actual deployed ODIS instances in staging, celo sepolia or mainnet environments.

To run all unit and integration tests (in both `/apps` and `/packages`) run

```bash
yarn test
```

To run ODIS e2e tests, navigate to the desired ODIS component subdirectory (either the Combiner or Signer) and run `yarn test:e2e`. You can specify the environment to run against as in the following example (see the relevant `package.json` file for all available test commands).

```bash
cd apps/signer
yarn test:e2e:celo-sepolia
```

For load tests, checkout [apps/monitor](../apps/monitor/README.md)

#### PRs and Releases

See [Release.md](../RELEASE.md) and [kubernetes-deployment](/kubernetes-deployment)
