{
  description = "Ground-up modular Rust radio platform development shell";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { nixpkgs, self, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
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
              renode
            ] ++ guiLibraries;

            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            DP32_LLD = "${pkgs.llvmPackages.lld}/bin/ld.lld";
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath guiLibraries;
          };
        });

      formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
    };
}
