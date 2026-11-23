import Activity from "@lucide/solid/icons/activity";
import ArrowLeft from "@lucide/solid/icons/arrow-left";
import ArrowRight from "@lucide/solid/icons/arrow-right";
import ArrowUp from "@lucide/solid/icons/arrow-up";
import ChevronDown from "@lucide/solid/icons/chevron-down";
import ChevronRight from "@lucide/solid/icons/chevron-right";
import Ellipsis from "@lucide/solid/icons/ellipsis";
import Menu from "@lucide/solid/icons/menu";
import MessageSquareText from "@lucide/solid/icons/message-square-text";
import Mic from "@lucide/solid/icons/mic";
import MicOff from "@lucide/solid/icons/mic-off";
import Monitor from "@lucide/solid/icons/monitor";
import Plug from "@lucide/solid/icons/plug";
import Plus from "@lucide/solid/icons/plus";
import Settings from "@lucide/solid/icons/settings";
import Sparkles from "@lucide/solid/icons/sparkles";
import X from "@lucide/solid/icons/x";
import { Dynamic } from "@solidjs/web";

const icons = {
  plugin: Plug,
  chat: MessageSquareText,
  settings: Settings,
  send: ArrowUp,
  plus: Plus,
  mic: Mic,
  micOff: MicOff,
  sparkles: Sparkles,
  activity: Activity,
  monitor: Monitor,
  arrowLeft: ArrowLeft,
  arrowRight: ArrowRight,
  chevronRight: ChevronRight,
  chevronDown: ChevronDown,
  menu: Menu,
  more: Ellipsis,
  close: X,
};

export function Icon(props: { name: keyof typeof icons; size?: number }) {
  return <Dynamic component={icons[props.name]} size={props.size ?? 20} strokeWidth={1.65} aria-hidden="true" />;
}


