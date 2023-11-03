# Kubernetes Deployment

ODIS Signer can be deployed in Kubernetes with a Helm chart.

## Helm chart

ODIS signer Helm chart templates are available [here](https://github.com/celo-org/charts/tree/main/charts/odis-signer). The chart is available through a public GCP Artifact Registry `oci://us-west1-docker.pkg.dev/devopsre/clabs-public-oci/odis-signer`.

Hence the ODIS signer can be deployed as follows in your Kubernetes cluster:

```bash
helm upgrade -install <RELEASE_NAME> oci://us-west1-docker.pkg.dev/devopsre/clabs-public-oci/odis-signer -n <NAMESPACE> --create-namespace -f <VALUES_FILE_PATH> [--version <VERSION>]
```

Notice the following:

- `<RELEASE_NAME>`: Name of the Helm release.
- `<NAMESPACE>`: Kubernetes namespace to deploy the Helm chart.
- `<VALUES_FILE_PATH>`: Path to your `values.yaml` file that defines the Helm deployment. You can use the examples under [./values] as a guide, but you will have to modify it according to your needs. You can find a table defining the values file [here](https://github.com/celo-org/charts/tree/main/charts/odis-signer#values).
- `<VERSION>`: The version of the Helm chart. If omitted, it will use the latest version (available in the [chart's README](https://github.com/celo-org/charts/tree/main/charts/odis-signer#odis-signer) as a GitHub badge).

## cLabs Developers

For cLabs developers, this section contains links and information for useful deployments.

> :warning: You will need to be a cLabs employee with appropiate permissions to access these links.

ODIS signer is deployed in the following clusters:

- Staging: cluster `integration-tests`
  - Signer0 in namespace `odis-signer0-staging` with this [`values-signer0-staging.yaml` file](./values/staging/values-signer0-staging.yaml).
    - URL: `http://odis-signer0-staging.odis-signer0-staging:3000`. :warning: This is a URL internal to the cluster (i.e. not accessible from outside)!!
  - Signer1 in namespace `odis-signer1-staging` with this [`values-signer1-staging.yaml` file](./values/staging/values-signer1-staging.yaml).
    - URL: `http://odis-signer1-staging.odis-signer1-staging:3000`. :warning: This is a URL internal to the cluster (i.e. not accessible from outside)!!
  - Signer2 in namespace `odis-signer2-staging` with this [`values-signer2-staging.yaml` file](./values/staging/values-signer2-staging.yaml).
    - URL: `http://odis-signer2-staging.odis-signer2-staging:3000`. :warning: This is a URL internal to the cluster (i.e. not accessible from outside)!!

### Modifying the deployment

There are 2 main ways to modify the ODIS combiner deployment in Kubernetes.

- Directly modify the deployment in the GCP console.
- Use [Helm](https://helm.sh/).

#### Directly modify the deployment in the GCP console

You can access the ODIS deployment by following these links. There you can edit the deployment and modify any value as needed (image, Env. Vars., etc.).

- Staging:
  - [Signer0](https://console.cloud.google.com/kubernetes/deployment/us-west1-b/integration-tests/odis-signer0-staging/odis-signer0-staging/yaml/view?project=celo-testnet&supportedpurview=project)
  - [Signer1](https://console.cloud.google.com/kubernetes/deployment/us-west1-b/integration-tests/odis-signer1-staging/odis-signer1-staging/yaml/view?project=celo-testnet&supportedpurview=project)
  - [Signer2](https://console.cloud.google.com/kubernetes/deployment/us-west1-b/integration-tests/odis-signer2-staging/odis-signer2-staging/yaml/view?project=celo-testnet&supportedpurview=project)

#### Use Helm

1. Ensure you are connected to the correct Kubernetes cluster (currently, only staging).
2. Get the currently deployed Helm chart values:

   ```bash
   helm get values -n odis-signer<0|1|2>-staging odis-signer<0|1|2>-staging -o yaml > ./values/values-signer<0|1|2>-staging.yaml
   ```

3. Modify the values file accordingly
4. Deploy the new release:

   ```bash
   helm upgrade -install odis-signer<0|1|2>-staging oci://us-west1-docker.pkg.dev/devopsre/clabs-public-oci/odis-signer -n odis-signer<0|1|2>-staging -f ./values/values-signer<0|1|2>-staging --create-namespace --version <VERSION>
   ```

5. Ensure there are no sensitive values in the values `.yaml` file and commit it to this repo.

### Postgres DB

Each signer has an associated Postgres DB running in its same Kubernetes namespace. These DBs are a copy of the DBs in Azure (database, tables, users, permissions, etc.).

- Staging:
  - Signer0 DB host: `odis-signer0-staging-db-postgresql-hl`, port 5432. :warning: This is a URL internal to the cluster (i.e. not accessible from outside)!!
    - Deployment: [`odis-signer0-staging-db-postgresql`](https://console.cloud.google.com/kubernetes/statefulset/us-west1-b/integration-tests/odis-signer0-staging/odis-signer0-staging-db-postgresql/details?project=celo-testnet&supportedpurview=project)
  - Signer1 DB host: `odis-signer1-staging-db-postgresql-hl`, port 5432. :warning: This is a URL internal to the cluster (i.e. not accessible from outside)!!
    - Deployment: [`odis-signer1-staging-db-postgresql`](https://console.cloud.google.com/kubernetes/statefulset/us-west1-b/integration-tests/odis-signer1-staging/odis-signer1-staging-db-postgresql/details?project=celo-testnet&supportedpurview=project)
  - Signer2 DB host: `odis-signer2-staging-db-postgresql-hl`, port 5432. :warning: This is a URL internal to the cluster (i.e. not accessible from outside)!!
    - Deployment: [`odis-signer2-staging-db-postgresql`](https://console.cloud.google.com/kubernetes/statefulset/us-west1-b/integration-tests/odis-signer2-staging/odis-signer2-staging-db-postgresql/details?project=celo-testnet&supportedpurview=project)

### Tracing

Tracing is enabled in the ODIS signer. The signers send traces to a Grafana Agent deployed in the same cluster as the signers.

- Staging Grafana Agent URL: `http://grafana-agent.monitoring:14268/api/traces`
