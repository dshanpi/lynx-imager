import type { ImageInspection, ImageReference } from '../../Platform/IPC/contracts/ImagerImage';
import type { BasicRoute } from './BasicPlan';

export const awRouteText: Partial<Record<BasicRoute, [string, string, string]>> = {
  vendor: ['安装厂商系统', '原厂完整烧录包', '按包内启动组件与分区表写入'],
  mainline_spinor: ['主线 · SPI NOR', '完整 SPI NOR 系统镜像', '完整镜像从物理起点写入'],
  mainline_nand: ['主线 · NAND', '主线 NAND 组件包', '按 Boot0、Boot1 与分区分别写入'],
  mainline_emmc: ['主线 · eMMC', '含主线 SPL 的整盘镜像', '从镜像提取 Boot0，另写 eMMC 用户区'],
};

export function awMainline(storage: string): BasicRoute | undefined {
  if (/nand/i.test(storage)) return 'mainline_nand';
  if (/nor/i.test(storage)) return 'mainline_spinor';
  if (/emmc/i.test(storage)) return 'mainline_emmc';
}

export function ramComponents(image: ImageInspection) {
  return [
    image.entries.find(
      (e) => e.mainType === 'FES' && e.subtype === 'FES_1-0000000000' && e.sizeBytes > 0
    ),
    image.entries.find(
      (e) => e.mainType === '12345678' && e.subtype === 'UBOOT_0000000000' && e.sizeBytes > 0
    ),
  ];
}

export function sourceProblem(route: BasicRoute, image: ImageInspection): string {
  const has = (...subtypes: string[]) =>
    image.entries.some((e) => e.sizeBytes > 0 && subtypes.includes(e.subtype));
  if (route === 'vendor' || route === 'mainline_nand') {
    if (!image.containerVersion)
      return '这条路线需要 IMAGEWTY 组件包，请核对选择的文件与系统用途。';
    if (!has('1234567890___MBR', '1234567890___GPT', 'SYS_CONFIG000000'))
      return '这个包没有可识别的分区布局，请检查是否误选了仅供 RAM 启动的引导包。';
    if (route === 'vendor' && !ramComponents(image).every(Boolean))
      return '原厂包未同时提供本工具使用的 FES 和 U-Boot 引导组件。';
    if (
      route === 'mainline_nand' &&
      !(
        has('BOOT0_0000000000', 'TOC0_NAND0000000') &&
        has('UBOOT_0000000000', 'TOC1_00000000000', 'BOOTPKG-00000000')
      )
    )
      return '未识别到 NAND Boot0 和 Boot1 组件，请选择板卡配套的 NAND 组件包。';
  } else if (route === 'mainline_spinor') {
    if (!image.egonAtZero || image.sizeBytes <= 0x80000 || image.containerVersion)
      return 'F101 需要物理起点带 eGON 启动头、覆盖 0x080000 应用区的完整 SPI NOR 镜像。';
  } else if (route === 'mainline_emmc' && !image.splAt8192) {
    return '没有在文件偏移 0x2000 找到此 eMMC 路线要求的 eGON.BT0 与 SPL 标记。';
  }
  return '';
}

export function referenceProblem(
  ref: ImageReference,
  image: ImageInspection,
  route: BasicRoute,
  board: string
): string {
  if (ref.sha256.toLowerCase() !== image.sha256.toLowerCase())
    return '镜像 SHA-256 与参考值不同，请更换系统镜像或选择正确的清单。';
  if (ref.sizeBytes !== null && ref.sizeBytes !== image.sizeBytes)
    return '清单中的文件大小与本地镜像不同。';
  if (ref.boardModel) {
    const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, '');
    const match =
      norm(ref.boardModel) === norm(board) ||
      (norm(board) === 'f101' &&
        ['yuzukineko', 'yuzukihdneko', 'f101s3'].includes(norm(ref.boardModel)));
    if (!match) return '清单声明的板型与当前选择不同：' + ref.boardModel;
  }
  if (
    route === 'mainline_spinor' &&
    ref.applicationOffset !== null &&
    ref.applicationOffset !== 0x80000
  )
    return '清单中的应用偏移与当前 F101 路线不符。';
  if (
    ref.applicationOffset !== null &&
    (ref.applicationOffset >= image.sizeBytes ||
      (ref.applicationBytes !== null &&
        ref.applicationBytes > image.sizeBytes - ref.applicationOffset))
  )
    return '清单声明的应用区超出了本地镜像范围。';
  return '';
}
