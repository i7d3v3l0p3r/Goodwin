import { Show } from "solid-js";
import styles from "./Brand.module.css";

export function SymbolMark(props: { class?: string }) {
  return <span class={[styles.mark, props.class]} aria-hidden="true"><i /><i /></span>;
}

export function Brand(props: { href?: string; class?: string }) {
  const content = () => <><SymbolMark /><span>Goodwin</span></>;
  const className = () => [styles.brand, props.class];

  return <Show when={props.href} fallback={<div class={className()}>{content()}</div>}>
    {(href) => <a class={className()} href={href()}>{content()}</a>}
  </Show>;
}
