import { qrcodegen } from "./vendor/qrcodegen.js";

export function invitationQr(code: string): string {
  const qr = qrcodegen.QrCode.encodeText(code, qrcodegen.QrCode.Ecc.MEDIUM);
  const modules: string[] = [];
  for (let row = 0; row < qr.size; row++) for (let column = 0; column < qr.size; column++) {
    if (qr.getModule(column, row)) modules.push(`M${column + 4},${row + 4}h1v1h-1z`);
  }
  return `<svg viewBox="0 0 ${qr.size + 8} ${qr.size + 8}" xmlns="http://www.w3.org/2000/svg" aria-hidden="true" class="remote-qr"><path class="remote-qr-paper" d="M0 0h${qr.size + 8}v${qr.size + 8}H0z"/><path class="remote-qr-ink" d="${modules.join("")}"/></svg>`;
}
