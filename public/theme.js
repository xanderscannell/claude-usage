/* Theme: dark, light, or whatever Windows uses. A classic script in
   <head>, so it runs before the first paint and a light panel never
   flashes dark on launch. "system" resolves here to light or dark, so
   the stylesheet only has to know those two. */
(() => {
  const KEY = "theme";
  const CHOICES = ["dark", "light", "system"];
  const system = matchMedia("(prefers-color-scheme: light)");
  // Until you pick one, follow Windows.
  let choice = "system";
  try {
    choice = localStorage.getItem(KEY) || "system";
  } catch {}
  if (!CHOICES.includes(choice)) choice = "system";

  function apply() {
    const light = choice === "light" || (choice === "system" && system.matches);
    document.documentElement.dataset.theme = light ? "light" : "dark";
    const btn = document.getElementById("theme");
    if (!btn) return;
    const next = CHOICES[(CHOICES.indexOf(choice) + 1) % CHOICES.length];
    btn.dataset.choice = choice;
    btn.setAttribute("aria-label", `Theme: ${choice}, switch to ${next}`);
    btn.title = `Theme: ${choice}`;
  }

  apply();
  system.addEventListener("change", apply);
  addEventListener("DOMContentLoaded", () => {
    apply();
    document.getElementById("theme").addEventListener("click", () => {
      choice = CHOICES[(CHOICES.indexOf(choice) + 1) % CHOICES.length];
      try {
        localStorage.setItem(KEY, choice);
      } catch {}
      apply();
    });
  });
})();
