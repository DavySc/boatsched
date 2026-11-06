# The SPA: `npm run build` (typecheck + Vite) produces one self-contained
# index.html, which the server embeds at compile time.
{ lib, buildNpmPackage, nodejs_22 }:

buildNpmPackage {
  pname = "boatsched-web";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ../web;
    fileset = lib.fileset.difference ../web (lib.fileset.maybeMissing ../web/node_modules);
  };
  nodejs = nodejs_22;
  # Bump with `nix run nixpkgs#prefetch-npm-deps web/package-lock.json` whenever
  # package-lock.json changes.
  npmDepsHash = "sha256-vJKge3b5EtLOAM57F7aheqfTo3RggCH2a55rHX7DSrY=";
  installPhase = ''
    runHook preInstall
    install -Dm644 dist/index.html $out/index.html
    runHook postInstall
  '';
}
