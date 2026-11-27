import { onSettled } from "solid-js";

export function TrackTitle(props: { title: string }) {
  let container!: HTMLSpanElement;
  let text!: HTMLSpanElement;

  onSettled(() => {
    const observer = new ResizeObserver(() => {
      const overflow = text.getBoundingClientRect().width - container.getBoundingClientRect().width;
      container.toggleAttribute("data-title-overflow", overflow > 0);
      if (overflow > 0) {
        container.style.setProperty("--title-overflow", `${overflow}px`);
        container.style.setProperty("--title-duration", `${Math.max(2, overflow / 30)}s`);
      }
    });
    observer.observe(container);
    observer.observe(text);
    return () => observer.disconnect();
  });

  return <span class="spotify-track-title" ref={container} title={props.title}>
    <span class="spotify-title">
      <span class="spotify-title-text" ref={text}>{props.title}</span>
    </span>
  </span>;
}
