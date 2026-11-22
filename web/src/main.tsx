import "./style.css";
import "./components/ui-kit/register";

import { render } from "@solidjs/web";
import { PanelLayout } from "./components/PanelLayout";
import { PanelRouter } from "./router";

render(() => (
  <PanelRouter>
    {(route) => <PanelLayout>{route.children}</PanelLayout>}
  </PanelRouter>
), document.getElementById("app")!);
