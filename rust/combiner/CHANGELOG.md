# Rust combiner changelog

Changes an operator sees when moving from the TS combiner (`apps/combiner`) to the Rust combiner.

## Migrating a deployment

Filled in with the last change to the combiner, when the full list is known.

## Configuration

The Rust combiner reads the TS combiner's variable names with the TS defaults. It always reads the
environment, where the TS combiner ignores it unless `NODE_ENV=production`, so there is no built-in
development configuration: the three required variables must be set for any run.

Required: `PNP_ODIS_SERVICES_SIGNERS`, `PNP_KEYS_VERSIONS` and `BLOCKCHAIN_PROVIDER`. The TS
combiner falls back to the chain's public RPC when `BLOCKCHAIN_PROVIDER` is unset; the Rust
combiner stops.

### Not supported

- TLS termination. `SERVER_SSL_KEY_PATH` and `SERVER_SSL_CERT_PATH` are not read. The combiner
  serves plain HTTP; terminate TLS in front of it.
- Trace export. `TRACER_ENDPOINT` and `TRACING_SERVICE_NAME` are not read.
- A signer's `fallbackUrl`. An entry that carries it is accepted, and the combiner calls a signer
  only at its `url`, never at the fallback.
- The no-quota cache. `PNP_SHOULD_CHECK_QUOTA` is not read, so the combiner never serves a
  `/quotaStatus` answer from a cache and never skips the signers.
- Domain endpoints. `DOMAINS_API_ENABLED` and the other `DOMAIN*` variables are not read.

### Variables without effect

These are not read, whatever their value, and nothing stops the start.

| Variable                           | Why it has no effect                                                                  |
| ---------------------------------- | ------------------------------------------------------------------------------------- |
| `PHONE_NUMBER_PRIVACY_API_ENABLED` | The PNP API is always on. Stop the combiner to take it down.                          |
| `PNP_SHOULD_AUTHENTICATE`          | The combiner always authenticates, so it always needs a full node.                    |
| `PNP_SHOULD_CHECK_QUOTA`           | There is no no-quota cache; see above.                                                |
| `PNP_SHOULD_MOCK_ACCOUNT_SERVICE`  | There is no mock account service. DEKs always come from the full node.                |
| `PNP_MOCK_DEK`                     | Same as above.                                                                        |

The Rust signer differs: it stops on its mock switches. The combiner does not, so a manifest that
sets them keeps starting but no longer mocks anything.

### Signer entries

`PNP_ODIS_SERVICES_SIGNERS` is a JSON list of objects with a string `url`. Other fields are
ignored. An entry may carry a `shareIndex`, which the TS combiner does not read. A partial
signature with another share index is then dropped. Indices count from 0, and one index applies to
every configured key version. A signer's index is the last four bytes of the base64-decoded
partial signature in its answer, which the TS combiner already logs.

### Refused at startup

Each refusal names the variable and says what to change.

- Two signer entries with the same `shareIndex`.
- A signer list or key version list that is not valid JSON of the expected shape.
- `PNP_ODIS_SERVICES_TIMEOUT_MILLISECONDS` or `PNP_FULL_NODE_TIMEOUT_MS` of 0.
- A key version whose `threshold` is 0 or above the number of signers. This is checked before the
  polynomial is decoded.
- A polynomial or public key that does not decode, or that does not encode back to the bytes it was
  decoded from. The decoder in the BLS library accepts trailing bytes, and the combiner does not.
- A key version whose `threshold` is not its polynomial's degree plus one. The Rust library takes
  the threshold from the polynomial, where TS passes the configured number, so the two must agree.
- A key version whose `pubKey` is the identity point or is not the constant term of its polynomial.
- Two entries with the same `keyVersion`. The TS combiner silently uses the first.
- A `PNP_KEYS_CURRENT_VERSION` that is not in the key version list.
- `BLOCKCHAIN_PROVIDER` unset or empty.
- `LOG_FORMAT` other than `text` or `json`. The TS combiner's `stackdriver` and `human` stop the
  combiner. `RUST_LOG` sets the log level.

### Numbers

A number that does not parse, such as `SERVER_PORT=abc`, stops the start. The TS combiner reads
numbers as `Number(value ?? default)`: it starts with `NaN` for `abc`, and it reads an empty string
as 0 and not as the default. The Rust combiner stops on an empty number too, including `CHAIN_ID`,
which TS reads as unset and then takes Celo Sepolia. An empty `LOG_FORMAT` is the one empty value
TS treats as unset (its default is `human`); the Rust combiner stops on it, and its default
is `text`. `PNP_FULL_NODE_RETRY_COUNT` counts retries after the first try, where TS counts all tries.
The whole full node lookup is capped at `PNP_FULL_NODE_TIMEOUT_MS`, as in the Rust signer.

## Request bodies

The combiner parses the client's JSON body and writes it back as `JSON.stringify` does: compact,
keys in the order sent, unknown fields kept. The signature check, the 16,000 character limit (UTF-16
code units of that form, so whitespace does not count) and the signer calls all use it. A body the
Rust writer cannot reproduce exactly as JavaScript would is refused with 400 `CELO_ODIS_WARN_01`,
where the TS combiner accepts it. Signers would otherwise check the signature over different
strings, and some would charge a request that then fails. Refused:

- A number that is not an integer from -(2^53 - 1) to 2^53 - 1, such as `1.0`, `1e2`, `-0`, `1e400`
  (which JavaScript writes as `null`) or a 20-digit integer.
- A key that appears twice in one object. JavaScript keeps the last value at the first key's place.
- A key made only of ASCII digits, in any object. JavaScript moves integer-like keys to the front.
  The combiner refuses every such key, a superset of those JavaScript moves.
- A lone surrogate escape such as `"\ud83d"`, which JavaScript accepts and writes back.
- A UTF-8 byte order mark before the body, an invalid UTF-8 byte (JavaScript reads it as U+FFFD),
  and nesting of 128 or more levels.

Transport differences, none of which touch a client that uses the SDK:

| Request | TS combiner | Rust combiner |
| --- | --- | --- |
| `Content-Type` is not JSON, or missing | Express skips the body; the handler sees an empty object and answers 400 `WARN_01` | 400 `WARN_01` |
| `charset` other than UTF-8 | `utf-*` is decoded and served; anything else gets 415 with an HTML body | 400 `WARN_01` |
| body with a `Content-Encoding` | inflated | not decoded, so 400 `WARN_01` |
| malformed JSON | 400 with an HTML body and no CORS headers | 400 `WARN_01` with the CORS headers |

## Request fields

- The `odis-key-version` header counts only when, after trimming whitespace, it is plain decimal
  digits. `1.0`, `0x1`, `0b1`, `0o1`, `1e0` and `+1`, which JavaScript's `Number()` reads as a
  version, count as absent, so the current version is used. A header sent twice counts as absent,
  as in TS, where Node joins the values and `Number()` reads NaN. A digit string beyond `u32` is an
  unsupported version (400 `WARN_12`); TS counts one above about 1.8e308 as absent, because
  `Number()` reads it as `Infinity`.
- An explicit `null` for `authenticationMethod`, `sessionID` or `version` is refused with 400
  `WARN_01`, as in TS. The Rust signer accepts it.

## Quota state

- When the signers' total quota disagrees, `combiner_warnings_total` counts `WARN_06`. The TS
  combiner counts `WARN_18` there, which is the label for a disagreement in the query count.
