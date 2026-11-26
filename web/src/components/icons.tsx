import Activity from "@lucide/solid/icons/activity";
import ArrowLeft from "@lucide/solid/icons/arrow-left";
import ArrowRight from "@lucide/solid/icons/arrow-right";
import ArrowUp from "@lucide/solid/icons/arrow-up";
import AudioLines from "@lucide/solid/icons/audio-lines";
import CalendarDays from "@lucide/solid/icons/calendar-days";
import ChevronDown from "@lucide/solid/icons/chevron-down";
import ChevronRight from "@lucide/solid/icons/chevron-right";
import Database from "@lucide/solid/icons/database";
import Download from "@lucide/solid/icons/download";
import Ellipsis from "@lucide/solid/icons/ellipsis";
import Headphones from "@lucide/solid/icons/headphones";
import House from "@lucide/solid/icons/house";
import ListChecks from "@lucide/solid/icons/list-checks";
import Menu from "@lucide/solid/icons/menu";
import MessageSquareText from "@lucide/solid/icons/message-square-text";
import Mic from "@lucide/solid/icons/mic";
import MicOff from "@lucide/solid/icons/mic-off";
import Monitor from "@lucide/solid/icons/monitor";
import Plug from "@lucide/solid/icons/plug";
import Plus from "@lucide/solid/icons/plus";
import Search from "@lucide/solid/icons/search";
import Settings from "@lucide/solid/icons/settings";
import Sparkles from "@lucide/solid/icons/sparkles";
import Timer from "@lucide/solid/icons/timer";
import Wifi from "@lucide/solid/icons/wifi";
import X from "@lucide/solid/icons/x";
import { Dynamic } from "@solidjs/web";

const icons = {
  plugin: Plug,
  chat: MessageSquareText,
  settings: Settings,
  search: Search,
  wifi: Wifi,
  send: ArrowUp,
  plus: Plus,
  mic: Mic,
  micOff: MicOff,
  sparkles: Sparkles,
  activity: Activity,
  monitor: Monitor,
  audioLines: AudioLines,
  calendarDays: CalendarDays,
  database: Database,
  headphones: Headphones,
  house: House,
  listChecks: ListChecks,
  timer: Timer,
  download: Download,
  arrowLeft: ArrowLeft,
  arrowRight: ArrowRight,
  chevronRight: ChevronRight,
  chevronDown: ChevronDown,
  menu: Menu,
  more: Ellipsis,
  close: X,
};

const pluginNavigationIcons = new Map<string, keyof typeof icons>([
  ["activity", "activity"],
  ["audio-lines", "audioLines"],
  ["calendar-days", "calendarDays"],
  ["database", "database"],
  ["download", "download"],
  ["headphones", "headphones"],
  ["house", "house"],
  ["list-checks", "listChecks"],
  ["message-square-text", "chat"],
  ["mic", "mic"],
  ["monitor", "monitor"],
  ["plug", "plugin"],
  ["settings", "settings"],
  ["sparkles", "sparkles"],
  ["timer", "timer"],
]);

export function PluginNavigationIcon(props: { name?: string }) {
  return <Icon name={pluginNavigationIcons.get(props.name ?? "") ?? "plugin"} />;
}

export function Icon(props: { name: keyof typeof icons; size?: number }) {
  return <Dynamic component={icons[props.name]} size={props.size ?? 20} strokeWidth={1.65} aria-hidden="true" />;
}


