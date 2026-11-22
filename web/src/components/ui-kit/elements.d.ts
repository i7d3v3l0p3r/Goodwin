import type { JSX } from "@solidjs/web/jsx-runtime";

declare module "@solidjs/web/jsx-runtime" {
  namespace JSX {
    interface IntrinsicElements {
      "gw-button": JSX.HTMLAttributes<HTMLElement> & {
        variant?: "primary" | "secondary";
        disabled?: boolean;
      };
      "gw-card": JSX.HTMLAttributes<HTMLElement>;
    }
  }
}
