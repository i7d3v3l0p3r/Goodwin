import { For } from "solid-js";

const strokes = {
  plugin: ["M10 3h4v5h4v5a6 6 0 0 1-5 6v2h-2v-2a6 6 0 0 1-5-6V8h4V3Zm-4 9h12"],
  chat: ["M5 18 3 20v-5a4 4 0 0 1-1-3V7a4 4 0 0 1 4-4h12a4 4 0 0 1 4 4v7a4 4 0 0 1-4 4H5Z", "M7 8h10M7 12h7"],
  settings: ["m10 3-.8 2.8-2.1.9-2.6-1.4-2 3.4 2.1 2v2.5l-2.1 2 2 3.4 2.6-1.4 2.1.9.8 2.9h4l.8-2.9 2.1-.9 2.6 1.4 2-3.4-2.1-2v-2.5l2.1-2-2-3.4-2.6 1.4-2.1-.9L14 3h-4Z", "M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z"],
  send: ["M12 19V5m-6 6 6-6 6 6"],
  plus: ["M12 5v14M5 12h14"],
  speaker: ["M8 9v6a4 4 0 0 0 8 0V9a4 4 0 0 0-8 0Zm-3 5v1a7 7 0 0 0 14 0v-1m-7 8v-2"],
  sparkles: ["M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9L12 3Zm7 12v6m-3-3h6M5 3v4M3 5h4"],
  activity: ["M2 12h5l3-7 4 14 3-7h5"],
  monitor: ["M3 4h18v13H3zM12 17v4m-4 0h8"],
  arrowLeft: ["M19 12H5m6-6-6 6 6 6"],
  arrowRight: ["M5 12h14m-6-6 6 6-6 6"],
  chevronRight: ["m9 6 6 6-6 6"],
  chevronDown: ["m6 9 6 6 6-6"],
};

export function Icon(props: { name: keyof typeof strokes; size?: number }) {
  return <svg width={props.size ?? 20} height={props.size ?? 20} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <For each={strokes[props.name]}>{(path) => <path d={path} />}</For>
  </svg>;
}


