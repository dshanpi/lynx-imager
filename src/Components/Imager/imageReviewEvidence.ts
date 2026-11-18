import { invokeCommand } from '../../Platform/IPC';

export interface ImageReviewEvidence {
  reference?: { path: string; sha256: string };
  application?: {
    imagePath: string;
    applicationPath: string;
    offset: number;
    expectedBytes: number | null;
    sha256: string;
  };
}

export async function recheckImageEvidence(evidence: ImageReviewEvidence | null) {
  if (!evidence) return;
  if (evidence.reference) {
    const actual = await invokeCommand('imager_read_image_reference', {
      path: evidence.reference.path,
    });
    if (actual.documentSha256 !== evidence.reference.sha256)
      throw new Error('镜像参考清单已改变，请返回选择系统并重新核验。');
  }
  if (evidence.application) {
    const { sha256, ...args } = evidence.application;
    const actual = await invokeCommand('imager_compare_application', args);
    if (!actual.matches || actual.applicationSha256 !== sha256)
      throw new Error('应用文件或镜像中的应用区已改变，请重新比对。');
  }
}
