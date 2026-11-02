# Contributing

Contributions are welcome. The project is open source under GPL-3.0-only, and the work happens on GitHub.

## 1. Fork and clone

1. Fork [marshalling-ltd/third-eye-client](https://github.com/marshalling-ltd/third-eye-client) with the **Fork** button on GitHub.
2. Clone your fork and add the original repository as `upstream`:

   ```sh
   git clone git@github.com:<your-user>/third-eye-client.git
   cd third-eye-client
   git remote add upstream git@github.com:marshalling-ltd/third-eye-client.git
   ```

## 2. Set up your machine

Follow [Building and testing](building.md#prerequisites), then check that everything works:

```sh
make requirements   # one-time tool setup
cargo run           # start the app
make check          # lint and full test run
```

No ROV at hand? Use the [ROV simulator](simulator.md).

## 3. Make a change

Keep your fork current, then work on a branch off `master`:

```sh
git fetch upstream
git switch -c my-change upstream/master
```

- Keep a change focused on one thing.
- Run `make check` before you push. CI runs the same format, clippy (pedantic, warnings denied), doc-test and `nextest` checks on macOS, Windows and Linux.
- Add or update tests for behaviour you change, and update the pages in `docs/` when the app's behaviour changes. [Preview the docs](#docs) locally.
- The history uses short messages in the form `client: what changed #issue`, for example `client: Bluetooth GPS pairing options Ubuntu #129`.

## 4. Open a pull request

Push the branch to your fork and open a pull request against `master` of `marshalling-ltd/third-eye-client`. Describe what changed and why, and link the issue it closes. CI must pass before a change is merged.

## Docs

The documentation lives in `docs/`, is built with [MkDocs Material](https://squidfunnel.github.io/mkdocs-material/) and is published to GitHub Pages when a change to it reaches `master`. To preview it:

```sh
uv run --with-requirements docs/requirements.txt mkdocs serve
```

Or with plain `pip`: `pip install -r docs/requirements.txt && mkdocs serve`. A pull request that touches `docs/` also runs a strict build, which fails on broken links.

## Reporting problems

Open an [issue](https://github.com/marshalling-ltd/third-eye-client/issues) with your OS, the app version (**Profile** → **Application updates**), what you expected and what happened.
