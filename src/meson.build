cargo_script = find_program(meson.project_source_root() / 'build-aux' / 'cargo.sh')

custom_target('cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: meson.project_name(),
  console: true,
  install: true,
  install_dir: bindir,
  command: [
    cargo_script,
    meson.project_build_root(),
    meson.project_source_root(),
    '@OUTPUT@',
    get_option('buildtype'),
    meson.project_name(),
    get_option('offline') ? 'offline' : 'online',
    prefix / get_option('localedir'),
  ],
)
