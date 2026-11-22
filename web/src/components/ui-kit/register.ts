import css from "./UiKit.module.css?inline";

const sheet = new CSSStyleSheet();
sheet.replaceSync(css);

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
