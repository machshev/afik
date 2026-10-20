{
  description = "Ground-up modular Rust radio platform development shell";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { nixpkgs, self, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      # nixpkgs no longer supports x86_64-darwin; the release still builds it.
      releaseSystems = systems ++ [ "aarch64-darwin" ];
      release = nixpkgs.lib.importJSON ./nix/release.json;
      forAllSystems = function:
        nixpkgs.lib.genAttrs systems (system: function nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forAllSystems (pkgs:
        let
          # `afik-studio` is an eframe/egui binary. Winit and glutin load the
          # window-system and GL libraries at run time, so they must be on the
          # loader path rather than only at link time.
          guiLibraries = with pkgs; [
            libGL
            libxkbcommon
            wayland
            libx11
            libxcursor
            libxi
            libxrandr
          ];
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              clippy
              rust-analyzer
              rustc
              rustfmt
              llvmPackages.bintools
              python3
              # The browser Studio (`ADR-119`): `trunk build` bundles it, and
              # wasm-bindgen-cli must match the locked wasm-bindgen exactly.
              trunk
              wasm-bindgen-cli
              binaryen
            ] ++ guiLibraries;

            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            DP32_LLD = "${pkgs.llvmPackages.lld}/bin/ld.lld";
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath guiLibraries;
          };
        });

      # The tools from the GitHub release named in `nix/release.json`:
      # `nix run github:machshev/afik` starts the studio, and
      # `nix shell github:machshev/afik` puts all four tools on the path.
      # Absent until the first release is pinned, so the flake still checks.
      packages = nixpkgs.lib.optionalAttrs (release.version != "") (nixpkgs.lib.genAttrs releaseSystems (system:
        let
          afik-studio = nixpkgs.legacyPackages.${system}.callPackage ./nix/afik-studio.nix { };
        in
        {
          inherit afik-studio;
          default = afik-studio;
        }));

      formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
    };
}
