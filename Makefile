# Build and publish Bayonet from this machine, the same way as every app on the release kit
# (scripts/release-kit/README.md, RELEASING.md). A release, from two machines:
#
#   (either)   make bump && git commit -am "Version x.y.z" && git push
#   (Windows)  git pull && make release     Windows installers, Linux in WSL, publish
#   (Mac)      git pull && make release     macOS Apple Silicon and Intel, publish
#
# Needs Node 22.18+, pnpm, Rust and git; on Windows, install make with
# `winget install ezwinports.make`.

KIT := node scripts/release-kit/release.mjs

.PHONY: help setup dev test check version bump installers linux linux-setup publish publish-dry-run release clean

## make setup            install dependencies
## make dev              run the app in development mode
## make test             Rust tests, type check and frontend tests
## make version          this checkout's version and what's published for each platform
## make bump             set the next version (published + 1, or VERSION=x.y.z), then commit it
## make installers       build this OS's installers into release/VERSION/
## make linux            (Windows) build the Linux installers in WSL Ubuntu-22.04
## make linux-setup      (Windows) install the Linux build tools in that WSL distro, once
## make publish          add release/VERSION/ to the download site and update feeds
## make publish-dry-run  build the site locally without pushing it
## make release          installers (+ linux on Windows) + publish + clean
## make clean            delete published builds, bundle output and dry-run sites
## ARGS=--force / --allow-dirty / --no-linux pass options to the release commands

# Prints the "## " lines above (works the same in cmd and sh).
help:
	@node -e "for (const l of require('fs').readFileSync('Makefile', 'utf8').split(String.fromCharCode(10))) if (l.startsWith('## ')) console.log(l.slice(3).trimEnd())"

setup:
	pnpm install

dev:
	pnpm tauri dev

test check:
	cargo nextest run --workspace && pnpm typecheck && pnpm test

version:
	$(KIT) version

bump:
	$(KIT) bump $(VERSION)

installers:
	$(KIT) build $(ARGS)

linux linux-setup publish release clean:
	$(KIT) $@ $(ARGS)

publish-dry-run:
	$(KIT) publish --dry-run $(ARGS)
