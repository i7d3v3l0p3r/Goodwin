import type { ComponentProps, JSX } from "@solidjs/web";
import { omit, Show, type ParentProps } from "solid-js";

type DetailsProps = Pick<ComponentProps<"details">, "class" | "name" | "open" | "ref" | "onToggle">;

export function Disclosure(props: ParentProps<DetailsProps & { title: string; description?: string; leading?: JSX.Element }>) {
  const attributes = omit(props, "class", "title", "description", "leading", "children");
  return <details {...attributes} class={["gw-disclosure", props.class]}>
    <summary>
      {props.leading}
      <span class="gw-disclosure-label">
        <span>{props.title}</span>
        <Show when={props.description}><small class="gw-disclosure-description">{props.description}</small></Show>
      </span>
    </summary>
    <div class="gw-disclosure-body">{props.children}</div>
  </details>;
}
