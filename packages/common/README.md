# ODIS Common Package

This package contains common code used across ODIS. It is depended on by the Combiner, Signer and Monitor services as well as the @celo/identity and @celo/encrypted-backup SDKS. In most cases where code will be re-used by multiple parts of ODIS, it probably belongs here.

## Notable Contents

- The request and response type schemas for all ODIS APIs.
- Error and Warning types used for monitoring and alerting in both the Combiner and Signer.
- The PEAR Sequential Delay rate limiting algorithm.


