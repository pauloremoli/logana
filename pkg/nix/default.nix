# Nix derivation for logana.
#
# Usage in a flake or overlay:
#
#   pkgs.callPackage ./pkg/nix/default.nix {}
#
# Or pin directly in your NixOS configuration / home-manager:
#
#   environment.systemPackages = [
#     (pkgs.callPackage (fetchFromGitHub {
#       owner = "pauloremoli";
#       repo  = "logana";
#       rev   = "v0.1.0";
#       hash  = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
#     } + "/pkg/nix") {})
#   ];
#
# To update the hashes after a version bump run:
#   nix-prefetch-url --unpack https://github.com/pauloremoli/logana/archive/vX.Y.Z.tar.gz
#   cargo vendor   # then re-run nix build to get the cargoHash

{ lib
, rustPlatform
, fetchFromGitHub
, pkg-config
, openssl
}:

rustPlatform.buildRustPackage rec {
  pname = "logana";
  version = "0.1.0";

  src = fetchFromGitHub {
    owner = "pauloremoli";
    repo  = "logana";
    rev   = "v${version}";
    # Run `nix-prefetch-url --unpack <tarball-url>` to obtain this value.
    hash  = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
  };

  # Run `cargo vendor` inside the source tree and compute the hash with
  # `nix hash path vendor/` to obtain this value.
  cargoHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ openssl ];

  # The test suite spins up a full TUI which requires a terminal; skip in
  # the Nix sandbox.
  doCheck = false;

  meta = with lib; {
    description = "A TUI log analyzer/viewer built for speed - handles files with millions of lines with instant filtering and VIM like navigation.";
    longDescription = ''
      logana is a terminal UI application for exploring, filtering, and
      annotating log files. It supports real-time search, persistent
      annotations, multiple tabs, colour themes, and session restore.
    '';
    homepage    = "https://github.com/pauloremoli/logana";
    license     = licenses.gpl3Only;
    maintainers = [ { name = "Paulo Remoli"; github = "pauloremoli"; } ];
    mainProgram = "logana";
    platforms   = platforms.linux ++ platforms.darwin;
  };
}
