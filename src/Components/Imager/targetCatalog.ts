export const mediaNames = ['SPI NOR', 'SPI NAND', 'eMMC', 'SD'] as const;
export type Medium = (typeof mediaNames)[number];
export type TargetChoice = { model: string; medium: Medium; reader?: boolean };

type Manufacturer = {
  id: string;
  name: string;
  brand: string;
  mark: string;
  tip: string;
  families: Record<string, string>;
};
export const manufacturers: Manufacturer[] = [
  {
    id: 'allwinner',
    name: '全志',
    brand: 'Allwinner',
    mark: 'AW',
    tip: '按主控表面的完整丝印查找型号，也可以搜索板名。',
    families: {
      T: 'T2 T3 T5 T7 T113 T153 T507 T517 T527',
      A: 'A10 A10s A13 A20 A23 A31 A31s A33 A40i A64 A80 A83T A100 A133 A523 A527',
      V: 'V3 V3s V5 V316 V536 V831 V833 V851S V853',
      H: 'H2+ H3 H5 H6 H8 H313 H616 H618 H700',
      F: 'F101 F1C100s F1C200s',
    },
  },
  {
    id: 'rockchip',
    name: '瑞芯微',
    brand: 'Rockchip',
    mark: 'RK',
    tip: '核对芯片型号末尾的字母。RK3588 与 RK3588S 分别列出。',
    families: {
      RK35: 'RK3576 RK3588 RK3506 RK3528 RK3562 RK3566 RK3568 RK3568J RK3588S',
      RK33: 'RK3308 RK3326 RK3328 RK3368 RK3399 RK3399Pro',
      RK32: 'RK3228 RK3229 RK3288',
      RK31: 'RK3126 RK3128 RK3188',
      RK30: 'RK3036 RK3066',
      RV11: 'RV1103 RV1103B RV1106 RV1106B RV1109 RV1126 RV1126B',
    },
  },
  {
    id: 'nxp',
    name: '恩智浦',
    brand: 'NXP',
    mark: 'NXP',
    tip: '以板卡配套的 UUU 脚本为准，核对脚本选用的存储设备。',
    families: { 'i.MX': 'i.MX6ULL i.MX8 i.MX9' },
  },
  {
    id: 'st',
    name: '意法半导体',
    brand: 'ST',
    mark: 'ST',
    tip: '按板级存储布局核对 DFU 接口及目标区域。',
    families: { STM32MP: 'STM32MP1 STM32MP157' },
  },
  {
    id: 'kendryte',
    name: '嘉楠',
    brand: 'Canaan',
    mark: 'CA',
    tip: '请按开发板实际装配的存储器选择目标，连接设备后再次核对。',
    families: { Kendryte: 'K230' },
  },
];

type Profile = { preset: string; media: Medium[]; aliases: string[]; routes: string };
export const profiles: Record<string, Profile | undefined> = {
  T113: {
    preset: 't113-pro',
    media: ['SPI NAND', 'eMMC'],
    aliases: ['T113-Pro'],
    routes: '厂商包 / 主线 NAND / 主线整盘',
  },
  T153: {
    preset: 't153-emmc',
    media: ['eMMC'],
    aliases: ['Omnigate T153'],
    routes: '厂商包 / 主线整盘',
  },
  F101: {
    preset: 'f101',
    media: ['SPI NOR', 'eMMC'],
    aliases: ['YuzukiNeko'],
    routes: '厂商包 / 主线整盘',
  },
  RK3576: {
    preset: 'rk3576',
    media: ['eMMC'],
    aliases: ['DshanPI-A1'],
    routes: 'update.img / 整盘',
  },
  RK3588: {
    preset: 'rk3588',
    media: ['eMMC'],
    aliases: ['RK3588-EVB1', 'Rock 5B (RK3588)'],
    routes: 'update.img / 整盘',
  },
  RK3588S: {
    preset: 'rk3588s',
    media: ['eMMC'],
    aliases: ['LubanCat-4 (RK3588S)'],
    routes: 'update.img / 整盘',
  },
  'i.MX6ULL': {
    preset: 'nxp-imx6ull',
    media: ['eMMC', 'SD'],
    aliases: ['i.MX 6ULL'],
    routes: '板级 UUU 配方',
  },
  'i.MX8': {
    preset: 'nxp-imx',
    media: ['eMMC', 'SD'],
    aliases: ['i.MX 8'],
    routes: '板级 UUU 配方',
  },
  'i.MX9': {
    preset: 'nxp-imx',
    media: ['eMMC', 'SD'],
    aliases: ['i.MX 9'],
    routes: '板级 UUU 配方',
  },
  STM32MP157: { preset: 'stm32mp157', media: ['eMMC', 'SD'], aliases: [], routes: 'DFU / DfuSe' },
  STM32MP1: { preset: 'stm32mp1', media: ['eMMC', 'SD'], aliases: [], routes: 'DFU / DfuSe' },
  K230: {
    preset: 'canaan-k230',
    media: [...mediaNames],
    aliases: ['CanMV-K230'],
    routes: 'KDImage / RAW',
  },
};
function vendorModels(vendor: string) {
  return Object.values(manufacturers.find((m) => m.id === vendor)!.families).flatMap((family) =>
    family.split(' ')
  );
}
export const rockchipModels = vendorModels('rockchip');
export const allwinnerModels = vendorModels('allwinner');
export function allwinnerEmmcPreset(model: string) {
  return model.toLowerCase() + '-emmc';
}
for (const model of allwinnerModels) {
  profiles[model] ??= {
    preset: allwinnerEmmcPreset(model),
    media: ['eMMC'],
    aliases: [],
    routes: '厂商包 / 主线整盘',
  };
}
for (const model of rockchipModels) {
  profiles[model] ??= {
    preset: model.toLowerCase(),
    media: ['eMMC'],
    aliases: [],
    routes: 'update.img / 整盘',
  };
}
export const models = manufacturers.flatMap((v) =>
  Object.entries(v.families).flatMap(([family, names]) =>
    names.split(' ').map((name) => ({ name, family, vendor: v.id, profile: profiles[name] }))
  )
);
export function searchModels(query: string) {
  const normalize = (s: string) => s.toLowerCase().replace(/[\s._-]/g, '');
  const needle = normalize(query);
  return models.filter((m) =>
    [m.name, ...(m.profile?.aliases ?? [])].some((alias) => normalize(alias).includes(needle))
  );
}
