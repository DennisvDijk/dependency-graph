# Releases and how to verify them

Releases are built and signed by the GitLab pipeline (`.gitlab/release.yml`), not by hand. Pushing a tag such as `v0.2.0` starts it:

```bash
git tag -s v0.2.0 -m "v0.2.0"     # -s signs the tag with your own key
git push origin v0.2.0
```

The pipeline builds the Linux binary, writes `SHA256SUMS` and a CycloneDX dependency list (`sbom.cdx.json`), signs each file and the container image with cosign, and creates the release with `glab release create`.

## What you get

| File | Purpose |
|---|---|
| `depgraph-x86_64-unknown-linux-gnu.tar.gz` | The binary |
| `SHA256SUMS` | Checksums of the files |
| `sbom.cdx.json` | Every crate that went into the binary |
| `<file>.bundle` | Signature and certificate for each file |
| `<registry>/<project>:<tag>` | Container image, signed by digest |

macOS binaries are not part of the pipeline: they need a macOS runner. Add a job like `build-linux` with that runner's tag when you have one.

## Verify before you run

Replace `GITLAB` and `GROUP/PROJECT` with your GitLab address and project path.

```bash
cosign verify-blob \
  --bundle depgraph-x86_64-unknown-linux-gnu.tar.gz.bundle \
  --certificate-identity-regexp '^https://GITLAB/GROUP/PROJECT//' \
  --certificate-oidc-issuer https://GITLAB \
  depgraph-x86_64-unknown-linux-gnu.tar.gz

cosign verify \
  --certificate-identity-regexp '^https://GITLAB/GROUP/PROJECT//' \
  --certificate-oidc-issuer https://GITLAB \
  REGISTRY/GROUP/PROJECT:v0.2.0

sha256sum -c SHA256SUMS
```

A pass means the file was produced by this project's pipeline for that tag and has not changed since. It does not say the code is free of bugs.

## Keyless or with a key

By default signing is keyless: GitLab hands the job a short-lived identity token and cosign exchanges it for a certificate through Sigstore's public services. That needs the runner to reach them.

A GitLab behind a firewall usually cannot. Two ways out:

1. Run your own Sigstore (Fulcio and Rekor) and point cosign at it.
2. Create a cosign key pair, store the private key and password as masked, protected CI/CD variables `COSIGN_PRIVATE_KEY` and `COSIGN_PASSWORD`, and publish the public key. The pipeline then signs with the key. Users verify with `--key cosign.pub` instead of the identity flags.

## Not covered yet

- The pipeline has not run on a real GitLab. Expect to fix a runner or network detail on the first tag; use a `v0.0.1-test` tag in a scratch project first.
- Base images in the Dockerfile are pinned by tag, not by digest.
- The older `.github/workflows/release.yml` still builds unsigned archives on GitHub. Delete it once GitLab is the only release path.
