/**
 * Tab 公共类型定义。
 *
 * 所有 Tab 都遵循统一接口：渲染一个 1-bit 位图到内部 Canvas，
 * 然后通过 onSend 回调把字节数组（SSD1306 page-major 格式）+ 预览 Canvas 交给父组件下发。
 *
 * 字节转换实现复用 Library/SSD1306/canvasToSSD1306Bytes，避免与字体渲染逻辑重复。
 */

import { canvasToSSD1306Bytes } from '../../../../Library/SSD1306';

/** 显示屏类型：决定推送通路。
 *  - monochrome-page：SSD1306 等 1-bit page 寻址屏，走 pushFramebuffer。
 *  - rgb565：ST7796 等 RGB565 屏，走 pushRgbRegion（CASET/RASET/RAMWR + 16bit）。 */
export type ScreenDisplayType = 'monochrome-page' | 'rgb565';

/** 给 4 个 Tab 通用的 props（父组件 MiddlePanel 传入）。 */
export interface TabContext {
  /** 屏幕宽度（像素）。来自硬件参数面板。 */
  width: number;
  /** 屏幕高度（像素）。已对齐到 8 的倍数（确保 SSD1306 page 整除）。 */
  height: number;
  /** Tab 触发"发送"时调用：父组件负责把 bytes 推到 SPI、把 canvas 推给 LCD 预览。 */
  onSend: (info: {
    bytes: number[];
    canvas: HTMLCanvasElement;
    description: string;
    bgColor?: string;
  }) => Promise<void>;
  /** 是否正在执行某个长任务（控件应禁用）。 */
  busy: boolean;
}

/**
 * 把 RGBA 像素 Canvas 转成 SSD1306 page-major 字节数组。
 * 直接复用 Library/SSD1306/canvasToSSD1306Bytes —— 此处仅作为别名导出，
 * 让 Tab 文件只导一个 common 模块就够。
 *
 * @param canvas 源 Canvas（高度需为 8 的倍数）
 * @param threshold 1-bit 阈值（0-255），默认 128
 */
export function canvasToBytes(canvas: HTMLCanvasElement, threshold = 128): number[] {
  return canvasToSSD1306Bytes(canvas, { threshold });
}
