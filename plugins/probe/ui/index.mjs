const messages = {
  en: { label: "Requests recorded", action: "Add one", error: "Could not reach the plugin." },
  ru: { label: "Записано запросов", action: "Прибавить один", error: "Не удалось обратиться к плагину." },
};

export function mount(container, { language, request }) {
  const text = messages[language];
  const card = document.createElement("gw-card");
  const content = document.createElement("div");
  content.className = "probe-content gw-stack";
  const count = document.createElement("p");
  count.setAttribute("role", "status");
  const button = document.createElement("gw-button");
  button.setAttribute("variant", "primary");
  button.textContent = text.action;
  content.append(count, button);
  card.append(content);
  container.append(card);

  async function load(method) {
    button.setAttribute("disabled", "");
    try {
      const result = await request("/counter", method);
      count.textContent = `${text.label} · ${result.count}`;
    } catch {
      count.textContent = text.error;
    } finally {
      button.removeAttribute("disabled");
    }
  }
  button.addEventListener("click", () => void load("POST"));
  void load("GET");
  return () => card.remove();
}
