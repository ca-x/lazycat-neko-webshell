export namespace qrcodegen {
  class QrCode {
    static Ecc: { MEDIUM: unknown };
    static encodeText(text: string, level: unknown): QrCode;
    readonly size: number;
    getModule(x: number, y: number): boolean;
  }
}
