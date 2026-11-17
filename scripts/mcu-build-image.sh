#!/usr/bin/env bash
set -euo pipefail

image_name="${LYNX_MCU_IMAGE:-lynx-mcu-build:ubuntu24.04}"
project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dockerfile_dir="${project_root}/docker/mcu-build"

docker_command=(docker)
if ! docker info >/dev/null 2>&1; then
  docker_command=(sudo docker)
fi

action="${1:-check}"
case "${action}" in
  build)
    "${docker_command[@]}" build \
      --tag "${image_name}" \
      "${dockerfile_dir}"
    ;;
  check)
    "${docker_command[@]}" run --rm "${image_name}" bash -lc '
      set -e
      arm-none-eabi-gcc --version | head -n 1
      cmake --version | head -n 1
      ninja --version
      openocd --version 2>&1 | head -n 1
      printf "int main(void) { for (;;) {} }\n" >/tmp/smoke.c
      arm-none-eabi-gcc -mcpu=cortex-m7 -mthumb -ffreestanding -c /tmp/smoke.c -o /tmp/smoke.o
      arm-none-eabi-size /tmp/smoke.o
    '
    ;;
  shell)
    "${docker_command[@]}" run --rm -it \
      --volume "${project_root}:/workspace" \
      --workdir /workspace \
      "${image_name}" bash
    ;;
  *)
    echo "usage: $0 {build|check|shell}" >&2
    exit 2
    ;;
esac
