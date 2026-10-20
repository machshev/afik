# The host tools and firmware images from a published GitHub release, so a
# Nix user runs exactly what the release page offers. `nix/release.json`
# names the release; `tool/update-release-nix.sh VERSION` fills it in.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  makeWrapper,
  libGL,
  libxkbcommon,
  wayland,
  libx11,
  libxcursor,
  libxi,
  libxrandr,
  releasePin ? lib.importJSON ./release.json,
  # Overrides for trying an archive that is not published yet.
  archiveOverride ? null,
  firmwareOverride ? null,
}:

let
  inherit (releasePin) version hashes;
  platforms = {
    x86_64-linux = "linux-x86_64";
    aarch64-linux = "linux-aarch64";
    aarch64-darwin = "macos-aarch64";
  };
  platform =
    platforms.${stdenv.hostPlatform.system}
      or (throw "afik-studio: no release package for ${stdenv.hostPlatform.system}");
  asset =
    name:
    fetchurl {
      url = "https://github.com/machshev/afik/releases/download/v${version}/${name}";
      hash =
        hashes.${name}
          or (throw "afik-studio: nix/release.json has no hash for ${name}; run tool/update-release-nix.sh");
    };
  archive = "afik-${version}-${platform}.tar.gz";
  images =
    if firmwareOverride != null then
      firmwareOverride
    else
      map asset [
        "afik-k5-v1-${version}.raw"
        "afik-k1-${version}.raw"
      ];
  # Winit and glutin load the window-system and GL libraries at run time.
  guiLibraries = [
    libGL
    libxkbcommon
    wayland
    libx11
    libxcursor
    libxi
    libxrandr
  ];
in
assert lib.assertMsg (version != "") "afik-studio: nix/release.json names no release yet";
stdenv.mkDerivation {
  pname = "afik-studio";
  inherit version;
  src = if archiveOverride != null then archiveOverride else asset archive;

  nativeBuildInputs = [
    makeWrapper
  ]
  ++ lib.optionals stdenv.hostPlatform.isLinux [ autoPatchelfHook ];
  buildInputs = lib.optionals stdenv.hostPlatform.isLinux [ stdenv.cc.cc.lib ];

  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    runHook preInstall
    install -Dm755 -t $out/bin afik-flasher afik-programmer afik-programmer-web afik-studio
    install -Dm644 -t $out/share/doc/afik README.md LICENSE
    mkdir -p $out/share/afik/firmware
    for image in ${lib.escapeShellArgs images}; do
      cp "$image" "$out/share/afik/firmware/$(stripHash "$image")"
    done
    runHook postInstall
  '';

  postFixup = lib.optionalString stdenv.hostPlatform.isLinux ''
    wrapProgram $out/bin/afik-studio \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath guiLibraries}
  '';

  meta = {
    description = "AFIK radio studio, programmer and flasher (released binaries)";
    homepage = "https://github.com/machshev/afik";
    license = lib.licenses.asl20;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
    platforms = builtins.attrNames platforms;
    mainProgram = "afik-studio";
  };
}
