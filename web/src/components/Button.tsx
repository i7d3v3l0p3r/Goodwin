import type { ComponentProps } from "@solidjs/web";
import { omit } from "solid-js";
import styles from "./Button.module.css";

export function Button(props: ComponentProps<"button"> & { variant?: "primary" | "secondary" }) {
  const attributes = omit(props, "class", "variant");
  return <button {...attributes} class={[styles.button, styles[props.variant ?? "secondary"], props.class]} />;
}
