{
  description = "Cranked Audio development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, fenix, ... }:
    let
      developmentSystems = [
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllDevelopmentSystems = nixpkgs.lib.genAttrs developmentSystems;
      pkgsFor = system: import nixpkgs { inherit system; };
    in
    {
      devShells = forAllDevelopmentSystems (
        system:
        let
          pkgs = pkgsFor system;
          rustToolchain = fenix.packages.${system}.stable.withComponents [
            "cargo"
            "clippy"
            "rust-src"
            "rustc"
            "rustfmt"
          ];
          runtimeLibraries = with pkgs; [
            alsa-lib
            dbus
            fontconfig
            freetype
            libGL
            libxkbcommon
            libpulseaudio
            udev
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
              rustToolchain
              fenix.packages.${system}.rust-analyzer
              bacon
              cargo-deny
              cargo-expand
              cargo-nextest
              actionlint
              git
              just
              nixfmt
              nushell
              statix
              taplo
              pkg-config
              gobject-introspection
              stdenv.cc
              curl
              jq
              pipewire
              pulseaudio
              systemd
              (python3.withPackages (ps: [
                ps.evdev
                ps.pygobject3
              ]))
            ];

            buildInputs = runtimeLibraries ++ [ pkgs.glib ];
            # Winit loads Wayland/X11 and keyboard libraries dynamically.
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibraries;
            CARGO_NET_GIT_FETCH_WITH_CLI = "true";
          };
        }
      );

      formatter = forAllDevelopmentSystems (system: (pkgsFor system).nixfmt);
    };
}
