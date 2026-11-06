# boatsched-server, linked against nixpkgs' source-built OR-Tools. The C++ shim
# is compiled against the same OR-Tools/abseil/protobuf headers as the library
# it links, so there is no prebuilt-SDK ABI to match.
{ lib, rustPlatform, or-tools, web }:

rustPlatform.buildRustPackage {
  pname = "boatsched-server";
  version = "0.1.0";

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../rust/Cargo.toml
      ../rust/Cargo.lock
      ../rust/crates
    ];
  };
  cargoRoot = "rust";
  buildAndTestSubdir = "rust";
  cargoLock.lockFile = ../rust/Cargo.lock;

  buildInputs = [ or-tools ];
  ORTOOLS_ROOT = "${or-tools}";

  # crates/server/build.rs embeds ../../../web/dist/index.html.
  preBuild = ''
    install -Dm644 ${web}/index.html web/dist/index.html
  '';

  # The workspace test suite stays the build gate: a failing test fails the
  # package, and therefore the deploy.
  cargoBuildFlags = [ "-p" "boatsched-server" ];
  cargoTestFlags = [ "--workspace" ];

  meta.mainProgram = "boatsched-server";
}
