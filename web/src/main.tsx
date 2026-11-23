import "./style.css";
import "./components/ui-kit/Button.css";
import "./components/ui-kit/Fields.css";
import "./components/ui-kit/Feedback.css";
import "./components/ui-kit/Disclosure.css";
import "./components/ui-kit/SaveActions.css";
import "./components/ui-kit/register";

import { render } from "@solidjs/web";
import { PanelLayout } from "./components/PanelLayout";
import { PanelRouter } from "./router";

render(() => (
  <PanelRouter>
    {(route) => <PanelLayout>{route.children}</PanelLayout>}
  </PanelRouter>
), document.getElementById("app")!);
