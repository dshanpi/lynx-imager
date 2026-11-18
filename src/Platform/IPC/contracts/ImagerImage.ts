export interface ImageInspection {
  knownF101Loader: boolean;
  sizeBytes: number;
  sha256: string;
  headerHex: string;
  appHex: string;
  egonAtZero: boolean;
  splAt8192: boolean;
  containerVersion: number | null;
  entries: Array<{
    name: string;
    mainType: string;
    subtype: string;
    sizeBytes: number;
    offset: number;
  }>;
}

export interface ImageReference {
  documentSha256?: string;
  sha256: string;
  sizeBytes: number | null;
  applicationOffset: number | null;
  applicationBytes: number | null;
  boardModel: string | null;
  fileName: string | null;
}

export interface AppComparison {
  matches: boolean;
  comparedBytes: number;
  applicationSha256: string;
  sampleHex: string;
}
