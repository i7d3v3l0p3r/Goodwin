import type { ComponentProps } from "@solidjs/web";
import { omit, Show, type ParentProps } from "solid-js";

type DetailsProps = Pick<ComponentProps<"details">, "class" | "name" | "open" | "ref" | "onToggle">;

export function Disclosure(props: ParentProps<DetailsProps & { title: string; description?: string }>) {
  const attributes = omit(props, "class", "title", "description", "children");
  return <details {...attributes} class={["gw-disclosure", props.class]}>
    <summary>
      <span class="gw-disclosure-label">
        <span>{props.title}</span>
        <Show when={props.description}><small class="gw-disclosure-description">{props.description}</small></Show>
      </span>
    </summary>
    {props.children}
  </details>;
}
