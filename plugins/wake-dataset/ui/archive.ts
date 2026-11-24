const block = 512;
const encode = new TextEncoder();

export function wavBytes(base64: string) {
  return Uint8Array.from(atob(base64), char => char.charCodeAt(0));
}

export class DatasetArchive {
  private parts: BlobPart[] = [];

  add(path: string, data: Uint8Array<ArrayBuffer>, capturedAtMs = 0) {
    const header = new Uint8Array(block);
    const string = (value: string, offset: number) => header.set(encode.encode(value), offset);
    const octal = (value: number, offset: number, width: number) => {
      string(value.toString(8).padStart(width - 1, "0"), offset);
    };
    if (encode.encode(path).length > 100) throw new Error("Archive path is too long");
    string(path, 0);
    octal(0o644, 100, 8);
    octal(0, 108, 8);
    octal(0, 116, 8);
    octal(data.length, 124, 12);
    octal(Math.floor(capturedAtMs / 1000), 136, 12);
    header.fill(32, 148, 156);
    string("0", 156);
    string("ustar\0" + "00", 257);
    octal(header.reduce((sum, byte) => sum + byte, 0), 148, 7);
    header[154] = 0;
    this.parts.push(header, data, new Uint8Array((block - data.length % block) % block));
  }

  download() {
    const blob = new Blob([...this.parts, new Uint8Array(2 * block)], { type: "application/x-tar" });
    const link = document.createElement("a");
    link.href = URL.createObjectURL(blob);
    link.download = `goodwin-wake-dataset-${new Date().toISOString().slice(0, 10)}.tar`;
    link.click();
    window.setTimeout(() => URL.revokeObjectURL(link.href), 30_000);
  }
}
