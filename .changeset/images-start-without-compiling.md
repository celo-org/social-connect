---
'@celo/phone-number-privacy-signer': patch
---

`yarn start:docker` and `yarn start:docker:tracing`, which the images and the Helm charts run, no longer compile TypeScript before they start the service. The image build has already compiled it, so a pod no longer spends a minute or two and up to 1.5 GiB of memory on `tsc` at every start. The images now keep only production dependencies, without TypeScript. Outside the image, run `yarn build` before `yarn start:docker`.
