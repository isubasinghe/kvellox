{
  description = "Kvellox development environment";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs = { nixpkgs, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
    in
    {
      devShells = nixpkgs.lib.genAttrs systems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          modelShell = (pkgs.mkShell.override {
            stdenv = pkgs.llvmPackages.libcxxStdenv;
          }) {
            packages = with pkgs; [ elan git curl unzip llvmPackages.lld python3 graphviz ];
            buildInputs = with pkgs; [ gmp libuv ];
            # CVC5's static archives need LLD's archive resolution on Linux.
            LEAN_CC = toString (pkgs.writeShellScript "lean-clang" ''
              exec ${pkgs.llvmPackages.libcxxStdenv.cc}/bin/clang \
                ${pkgs.lib.optionalString pkgs.stdenv.isLinux "-fuse-ld=lld"} "$@"
            '');
          };
        in
        {
          default = modelShell;
          model = modelShell;
        });
    };
}
