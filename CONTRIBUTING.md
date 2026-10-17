# Contributing to Loose Screws

Thank you for helping. Before your first contribution can be merged, please read and accept the
contributor license agreement below.

## How to contribute

- Open an issue first for anything larger than a small fix, so the approach can be agreed on.
- Read [docs/dev-guide.md](docs/dev-guide.md): building, running, testing and the full check.
  Every change follows [docs/code-quality.md](docs/code-quality.md) (one source for every fact,
  one definition per concept, no duplicated logic) and, for the simulation and the world
  generator, [docs/architecture/determinism.md](docs/architecture/determinism.md).
- Add or update tests next to the code you change, keep zero warnings, and keep the docs current
  in the same change.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/):
  `<type>(<scope>): <subject>`, imperative and lowercase, first line at most 72 characters, with a
  body that says what changed and why. The `commit-msg` hook checks them
  ([docs/adr/0002-public-repository.md](docs/adr/0002-public-repository.md)).
- Verification is local: run the full check before you open a pull request, `tools/ci.sh`
  (Linux, macOS) or `pwsh tools/ci.ps1` (Windows). There is no hosted CI
  ([docs/adr/0004-local-verification.md](docs/adr/0004-local-verification.md)).
- Dependencies must not be under the GPL, LGPL or AGPL (`cargo deny check licenses`,
  `node tools/check-licenses.mjs`).

## Why there is an agreement

The code of Loose Screws is licensed under the GNU Affero General Public License v3.0 or later
([LICENSE](LICENSE)); the game content (the data in `data/`, the media in `docs/media/`, the
game design document) under Creative Commons BY-NC-SA 4.0 ([LICENSE-CONTENT](LICENSE-CONTENT)).
The maintainer also offers Loose Screws under other terms, for example a commercial edition with
closed additions, and needs the right to include every contribution in both. The agreement keeps
your copyright with you and grants that right; it does not take away anything you may do with
your own contribution. Details: [docs/adr/0003-licensing.md](docs/adr/0003-licensing.md).

## Contributor license agreement

By submitting a contribution (a pull request, a patch or any other material sent for inclusion in
Loose Screws), you agree to the following terms for that contribution and every later one.

1. **Definitions.** "You" are the person or the legal entity submitting a contribution.
   "Contribution" is any original work of authorship, including changes and additions to existing
   work, code as well as content, that you submit for inclusion in Loose Screws. "Maintainer" is
   the person who holds the copyright in Loose Screws and publishes it, and anyone to whom the
   maintainer transfers the project.

2. **Copyright license.** You grant the maintainer a perpetual, worldwide, non-exclusive,
   no-charge, royalty-free, irrevocable license to use, reproduce, prepare derivative works of,
   publicly display, publicly perform, sublicense and distribute your contributions and such
   derivative works, under any terms the maintainer chooses: open-source and content licenses
   (including, but not limited to, the GNU Affero General Public License and Creative Commons
   licenses) as well as proprietary or commercial licenses.

3. **Patent license.** You grant the maintainer and everyone who receives Loose Screws from the
   maintainer a perpetual, worldwide, non-exclusive, no-charge, royalty-free, irrevocable patent
   license to make, have made, use, offer to sell, sell, import and otherwise transfer your
   contributions, where such a license is needed for patent claims you can license that are
   necessarily infringed by your contribution alone or by its combination with Loose Screws.

4. **Your rights stay yours.** You keep the copyright in your contributions and may use them in
   any other way you like. The maintainer is not obliged to use or publish a contribution.

5. **What you confirm.** Each contribution is your original work, or you have the right to submit
   it under these terms (for example, your employer has agreed, or it is material under a license
   that allows it, which you name in the submission). To your knowledge it does not infringe the
   rights of others. If you submit material of third parties, you say so and name its license.

6. **No warranty.** Unless required by law, you provide your contributions "as is", without
   warranties or conditions of any kind.

7. **Agreement.** You accept these terms by submitting a contribution and by confirming it in the
   pull request (a comment such as "I have read the contributor license agreement in
   CONTRIBUTING.md and accept it", or the signature requested by the repository's CLA tool, if one
   is set up).
