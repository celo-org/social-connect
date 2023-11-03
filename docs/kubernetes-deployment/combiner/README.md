# Kubernetes Deployment

ODIS Combiner can be deployed in Kubernetes with a Helm chart.

## Helm chart

ODIS combiner Helm chart templates are available [here](https://github.com/celo-org/charts/tree/main/charts/odis-combiner). The chart is available through a public GCP Artifact Registry `oci://us-west1-docker.pkg.dev/devopsre/clabs-public-oci/odis-combiner`.

Hence the ODIS combiner can be deployed as follows in your Kubernetes cluster:

```bash
helm upgrade -install <RELEASE_NAME> oci://us-west1-docker.pkg.dev/devopsre/clabs-public-oci/odis-combiner -n <NAMESPACE> --create-namespace -f <VALUES_FILE_PATH> [--version <VERSION>]
```

Notice the following:

- `<RELEASE_NAME>`: Name of the Helm release.
- `<NAMESPACE>`: Kubernetes namespace to deploy the Helm chart.
- `<VALUES_FILE_PATH>`: Path to your `values.yaml` file that defines the Helm deployment. You can use the examples under [./values] as a guide, but you will have to modify it according to your needs. You can find a table defining the values file [here](https://github.com/celo-org/charts/tree/main/charts/odis-combiner#values).
- `<VERSION>`: The version of the Helm chart. If omitted, it will use the latest version (available in the [chart's README](https://github.com/celo-org/charts/tree/main/charts/odis-combiner#odis-combiner) as a GitHub badge).

## cLabs Developers

For cLabs developers, this section contains links and information for useful deployments.

> :warning: You will need to be a cLabs employee with appropiate permissions to access these links.

ODIS combiner is deployed in the following clusters:

- Staging: cluster `integration-tests`, namespace `odis-combiner-staging` with this [`values-staging.yaml` file](./values/values-staging.yaml).
  - URL: `https://odis-combiner-staging.integration-tests.celo-networks-dev.org`
- Alfajores: cluster `alfajores`, namespace `odis-combiner-alfajores` with this [`values-alfajores.yaml` file](./values/values-alfajores.yaml).
  - URL: `https://odis-combiner-alfajores.alfajores.celo-testnet.org`
- Mainnet: cluster `rc1-us-west1`, namespace `odis-combiner-mainnet` with this [`values-mainnet.yaml` file](./values/values-mainnet.yaml).
  - URL: `https://odis-combiner-mainnet.rc1-europe-west1.celo-testnet.org`

### Modifying the deployment

There are 2 main ways to modify the ODIS combiner deployment in Kubernetes.

- Directly modify the deployment in the GCP console.
- Use [Helm](https://helm.sh/).

#### Directly modify the deployment in the GCP console

You can access the ODIS deployment by following these links. There you can edit the deployment and modify any value as needed (image, Env. Vars., etc.).

- [Staging](https://console.cloud.google.com/kubernetes/deployment/us-west1-b/integration-tests/odis-combiner-staging/odis-combiner-staging/yaml/view?project=celo-testnet&supportedpurview=project)
- [Alfajores](https://console.cloud.google.com/kubernetes/deployment/us-west1-a/alfajores/odis-combiner-alfajores/odis-combiner-alfajores/yaml/view?project=celo-testnet-production&supportedpurview=project)
- [Mainnet](https://console.cloud.google.com/kubernetes/deployment/europe-west1-b/rc1-europe-west1/odis-combiner-mainnet/odis-combiner-mainnet/yaml/view?project=celo-testnet-production&supportedpurview=project)

#### Use Helm

1. Ensure you are connected to the correct Kubernetes cluster (staging, alfajores or mainnet).
2. Get the currently deployed Helm chart values:

   ```bash
   helm get values -n odis-combiner-<staging|alfajores|mainnet> odis-combiner-<staging|alfajores|mainnet> -o yaml > ./values/values-<staging|alfajores|mainnet>.yaml
   ```

3. Modify the values file accordingly
4. Deploy the new release:

   ```bash
   helm upgrade -install odis-combiner-<staging|alfajores|mainnet> oci://us-west1-docker.pkg.dev/devopsre/clabs-public-oci/odis-combiner -n odis-combiner-<staging|alfajores|mainnet> -f ./values/values-<staging|alfajores|mainnet>.yaml --create-namespace --version <VERSION>
   ```

5. Ensure there are no sensitive values in the `./values/values-<staging|alfajores|mainnet>.yaml` file and commit it to this repo.

### Tracing

Tracing is enabled in the ODIS combiner. The combiners send traces to a Grafana Agent deployed in the same cluster as the combiners.

- Staging Grafana Agent URL: `http://grafana-agent.monitoring:14268/api/traces`
