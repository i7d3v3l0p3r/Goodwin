import buttonCss from "./Button.css?inline";
import cardCss from "./UiKit.module.css?inline";

const sheet = new CSSStyleSheet();
sheet.replaceSync(buttonCss + "\n" + cardCss);

function shadow(host: HTMLElement) {
  const root = host.attachShadow({ mode: "open" });
  root.adoptedStyleSheets = [sheet];
  return root;
}

class GoodwinButton extends HTMLElement {
  static observedAttributes = ["disabled", "variant"];
  #button = document.createElement("button");

  constructor() {
    super();
    this.#button.type = "button";
    this.#button.className = "gw-button";
    this.#button.append(document.createElement("slot"));
    shadow(this).append(this.#button);
  }

  attributeChangedCallback() {
    this.#button.disabled = this.hasAttribute("disabled");
    this.#button.dataset.variant = this.getAttribute("variant") ?? "secondary";
  }
}

class GoodwinCard extends HTMLElement {
  constructor() {
    super();
    const section = document.createElement("section");
    section.append(document.createElement("slot"));
    shadow(this).append(section);
  }
}

customElements.define("gw-button", GoodwinButton);
customElements.define("gw-card", GoodwinCard);
