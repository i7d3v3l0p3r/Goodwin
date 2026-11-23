import type { ComponentProps } from "@solidjs/web";
import { omit } from "solid-js";

type Variant = "primary" | "secondary" | "quiet";

export function Button(props: ComponentProps<"button"> & { variant?: Variant }) {
  const attributes = omit(props, "class", "variant");
  return <button {...attributes} class={["gw-button", props.class]} data-variant={props.variant} />;
}

export function ButtonLink(props: ComponentProps<"a"> & { href: string; variant?: Variant }) {
  const attributes = omit(props, "class", "variant");
  return <a {...attributes} class={["gw-button", props.class]} data-variant={props.variant} />;
}
