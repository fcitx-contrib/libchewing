let
  pkgs = import <nixpkgs> { config = {}; overlays = []; };
in

pkgs.mkShell {
  packages = with pkgs; [
    cargo
    rustc
    rustfmt
    rust-analyzer
    cmake
    ninja
    ncurses
    valgrind
    lldb
    clang-tools
  ];
}
