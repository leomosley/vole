// Client-side "peak 2003 malware" engine: synthesized sound effects, logos that
// fly across the screen, and self-replicating popup windows. No asset files, no
// external libraries. Everything degrades quietly if the browser blocks it.

type SfxName =
  "click" | "error" | "ding" | "dialup" | "alarm" | "coin" | "boing";

type Vec2 = { x: number; y: number };

const rand = (min: number, max: number): number =>
  min + Math.random() * (max - min);

const pick = <T>(items: readonly T[], fallback: T): T => {
  const index = Math.floor(rand(0, items.length));
  return items[index] ?? fallback;
};

// --- Sound -----------------------------------------------------------------

let audioContext: AudioContext | null = null;
let soundEnabled = true;

const getContext = (): AudioContext | null => {
  if (audioContext) return audioContext;
  const Ctor =
    window.AudioContext ??
    (window as unknown as { webkitAudioContext?: typeof AudioContext })
      .webkitAudioContext;
  if (!Ctor) return null;
  audioContext = new Ctor();
  return audioContext;
};

type ToneSpec = {
  freq: number;
  duration: number;
  type: OscillatorType;
  gain: number;
  sweepTo?: number;
  delay: number;
};

const playTone = (context: AudioContext, spec: ToneSpec): void => {
  const start = context.currentTime + spec.delay;
  const osc = context.createOscillator();
  const amp = context.createGain();
  osc.type = spec.type;
  osc.frequency.setValueAtTime(spec.freq, start);
  if (spec.sweepTo !== undefined) {
    osc.frequency.exponentialRampToValueAtTime(
      spec.sweepTo,
      start + spec.duration,
    );
  }
  amp.gain.setValueAtTime(0.0001, start);
  amp.gain.exponentialRampToValueAtTime(spec.gain, start + 0.01);
  amp.gain.exponentialRampToValueAtTime(0.0001, start + spec.duration);
  osc.connect(amp).connect(context.destination);
  osc.start(start);
  osc.stop(start + spec.duration + 0.02);
};

const recipes: Record<SfxName, (context: AudioContext) => void> = {
  click: (c) =>
    playTone(c, {
      freq: 900,
      duration: 0.05,
      type: "square",
      gain: 0.15,
      delay: 0,
    }),
  ding: (c) => {
    playTone(c, {
      freq: 880,
      duration: 0.12,
      type: "sine",
      gain: 0.2,
      delay: 0,
    });
    playTone(c, {
      freq: 1320,
      duration: 0.25,
      type: "sine",
      gain: 0.18,
      delay: 0.08,
    });
  },
  error: (c) => {
    playTone(c, {
      freq: 220,
      duration: 0.18,
      type: "square",
      gain: 0.2,
      delay: 0,
    });
    playTone(c, {
      freq: 180,
      duration: 0.28,
      type: "square",
      gain: 0.2,
      delay: 0.16,
    });
  },
  coin: (c) => {
    playTone(c, {
      freq: 988,
      duration: 0.08,
      type: "square",
      gain: 0.18,
      delay: 0,
    });
    playTone(c, {
      freq: 1319,
      duration: 0.3,
      type: "square",
      gain: 0.18,
      delay: 0.08,
    });
  },
  boing: (c) =>
    playTone(c, {
      freq: 500,
      duration: 0.35,
      type: "sine",
      gain: 0.22,
      sweepTo: 120,
      delay: 0,
    }),
  alarm: (c) => {
    for (let i = 0; i < 4; i += 1) {
      playTone(c, {
        freq: i % 2 === 0 ? 740 : 494,
        duration: 0.16,
        type: "sawtooth",
        gain: 0.16,
        delay: i * 0.18,
      });
    }
  },
  dialup: (c) => {
    const steps: ReadonlyArray<[number, number]> = [
      [0, 0.2],
      [0.25, 0.2],
      [0.5, 0.15],
    ];
    steps.forEach(([delay, dur]) =>
      playTone(c, {
        freq: rand(400, 700),
        duration: dur,
        type: "sine",
        gain: 0.12,
        delay,
      }),
    );
    playTone(c, {
      freq: 1100,
      duration: 0.9,
      type: "sawtooth",
      gain: 0.06,
      sweepTo: 2400,
      delay: 0.7,
    });
    playTone(c, {
      freq: 2100,
      duration: 0.9,
      type: "square",
      gain: 0.05,
      sweepTo: 900,
      delay: 0.7,
    });
  },
};

export const playSfx = (name: SfxName): void => {
  if (!soundEnabled) return;
  const context = getContext();
  if (!context) return;
  if (context.state === "suspended") void context.resume();
  recipes[name](context);
};

const setSound = (on: boolean): void => {
  soundEnabled = on;
  document.querySelectorAll<HTMLElement>("[data-sound-state]").forEach((el) => {
    el.textContent = on ? "SOUND: ON" : "SOUND: OFF";
  });
};

// --- Flying logos ----------------------------------------------------------

type Flyer = {
  el: HTMLImageElement;
  pos: Vec2;
  vel: Vec2;
  spin: number;
  angle: number;
};

const flyers: Flyer[] = [];

const spawnFlyer = (): void => {
  if (flyers.length > 14) return;
  const el = document.createElement("img");
  el.src = "/vole.png";
  el.alt = "VOLE";
  el.className = "chaos-flyer";
  const size = rand(48, 104);
  el.style.width = `${size}px`;
  el.style.height = `${size}px`;
  document.body.appendChild(el);
  flyers.push({
    el,
    pos: {
      x: rand(0, window.innerWidth - size),
      y: rand(0, window.innerHeight - size),
    },
    vel: { x: rand(-2.4, 2.4) || 1.6, y: rand(-2.4, 2.4) || 1.6 },
    spin: rand(-3, 3),
    angle: 0,
  });
};

const stepFlyers = (): void => {
  const w = window.innerWidth;
  const h = window.innerHeight;
  flyers.forEach((f) => {
    const size = f.el.offsetWidth;
    f.pos.x += f.vel.x;
    f.pos.y += f.vel.y;
    if (f.pos.x <= 0 || f.pos.x >= w - size) {
      f.vel.x *= -1;
      f.pos.x = Math.max(0, Math.min(f.pos.x, w - size));
    }
    if (f.pos.y <= 0 || f.pos.y >= h - size) {
      f.vel.y *= -1;
      f.pos.y = Math.max(0, Math.min(f.pos.y, h - size));
    }
    f.angle += f.spin;
    f.el.style.transform = `translate(${f.pos.x}px, ${f.pos.y}px) rotate(${f.angle}deg)`;
  });
  window.requestAnimationFrame(stepFlyers);
};

// --- Popups ----------------------------------------------------------------

type PopupSpec = { title: string; body: string; cta: string; sfx: SfxName };

const popupSpecs: readonly PopupSpec[] = [
  {
    title: "\u26A0 CONGRATULATIONS!!! \u26A0",
    body: "You are visitor #1,000,000! You have been selected to receive a FREE copy of VOLE\u2122. Click now before this offer self-destructs!",
    cta: "CLAIM MY VOLE",
    sfx: "coin",
  },
  {
    title: "SYSTEM SECURITY ALERT",
    body: "Your PC may not be running VOLE\u2122. Computers without VOLE are 400% louder than they should be. Install immediately to remain safe.",
    cta: "FIX IT NOW",
    sfx: "alarm",
  },
  {
    title: "\uD83D\uDD25 HOT SINGLES (audio sessions) IN YOUR AREA",
    body: "5 unmuted applications are BLASTING near you right now. VOLE\u2122 can duck them in one keypress. Do not keep them waiting.",
    cta: "MEET THEM",
    sfx: "ding",
  },
  {
    title: "SmartScreen? never heard of her",
    body: "Windows says 'unknown publisher'. That's just Windows being shy. VOLE\u2122 is a lightweight tray app, pinky promise. Click More info \u2192 Run anyway.",
    cta: "RUN ANYWAY",
    sfx: "boing",
  },
  {
    title: "\uD83D\uDCBE DOWNLOAD ACCELERATOR 2003",
    body: "Your download speed can be improved by 9000%. Simply install VOLE\u2122 and press some hotkeys. Results not guaranteed. Actually download VOLE.",
    cta: "ACCELERATE",
    sfx: "dialup",
  },
];

const defaultPopup: PopupSpec = {
  title: "\u26A0 IMPORTANT MESSAGE \u26A0",
  body: "VOLE\u2122 is ready to be installed. This message will not close (it will).",
  cta: "OK, FINE",
  sfx: "ding",
};

let popupCount = 0;
let popupZ = 4000;

const releaseUrl = "https://github.com/leomosley/vole/releases/latest";

const spawnPopup = (spec: PopupSpec = pick(popupSpecs, defaultPopup)): void => {
  if (popupCount >= 2) return;
  popupCount += 1;
  playSfx(spec.sfx);

  const win = document.createElement("div");
  win.className = "chaos-popup";
  win.style.left = `${rand(4, Math.max(4, window.innerWidth - 360))}px`;
  win.style.top = `${rand(4, Math.max(4, window.innerHeight - 260))}px`;
  popupZ += 1;
  win.style.zIndex = String(popupZ);

  const hue = Math.floor(rand(0, 360));
  win.style.setProperty("--pop-hue", String(hue));

  win.innerHTML = `
    <div class="chaos-popup__bar">
      <span class="chaos-popup__title">${spec.title}</span>
      <button class="chaos-popup__x" type="button" aria-label="close">X</button>
    </div>
    <div class="chaos-popup__body">
      <img src="/vole.png" alt="" class="chaos-popup__mascot" />
      <p>${spec.body}</p>
      <a class="chaos-popup__cta" href="${releaseUrl}" target="_blank" rel="noopener">${spec.cta}</a>
    </div>`;

  const close = win.querySelector<HTMLButtonElement>(".chaos-popup__x");
  close?.addEventListener("click", () => {
    playSfx("error");
    win.remove();
    popupCount -= 1;
  });

  const cta = win.querySelector<HTMLAnchorElement>(".chaos-popup__cta");
  cta?.addEventListener("click", () => playSfx("coin"));

  makeDraggable(win, win.querySelector<HTMLElement>(".chaos-popup__bar"));
  document.body.appendChild(win);
};

const makeDraggable = (win: HTMLElement, handle: HTMLElement | null): void => {
  if (!handle) return;
  let start: Vec2 | null = null;
  let origin: Vec2 = { x: 0, y: 0 };
  const onMove = (e: PointerEvent): void => {
    if (!start) return;
    win.style.left = `${origin.x + (e.clientX - start.x)}px`;
    win.style.top = `${origin.y + (e.clientY - start.y)}px`;
  };
  const onUp = (): void => {
    start = null;
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
  };
  handle.addEventListener("pointerdown", (e) => {
    start = { x: e.clientX, y: e.clientY };
    origin = { x: win.offsetLeft, y: win.offsetTop };
    popupZ += 1;
    win.style.zIndex = String(popupZ);
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  });
};

// --- Wiring ----------------------------------------------------------------

const armOnce = (start: () => void): void => {
  const handler = (): void => {
    start();
    window.removeEventListener("pointerdown", handler);
    window.removeEventListener("keydown", handler);
  };
  window.addEventListener("pointerdown", handler, { once: true });
  window.addEventListener("keydown", handler, { once: true });
};

export const initChaos = (): void => {
  // Global click blips on anything that looks interactive.
  document.addEventListener("click", (e) => {
    const target = e.target as HTMLElement | null;
    if (target?.closest("a, button, [data-blip]")) playSfx("click");
  });

  // Sound toggle buttons.
  document
    .querySelectorAll<HTMLElement>("[data-sound-toggle]")
    .forEach((el) => {
      el.addEventListener("click", () => setSound(!soundEnabled));
    });

  // Manual popup trigger buttons.
  document.querySelectorAll<HTMLElement>("[data-popup]").forEach((el) => {
    el.addEventListener("click", () => spawnPopup());
  });

  window.requestAnimationFrame(stepFlyers);
  for (let i = 0; i < 4; i += 1) spawnFlyer();
  window.setInterval(spawnFlyer, 6000);

  // The chaos only truly begins after the first interaction (browser audio policy).
  armOnce(() => {
    playSfx("dialup");
    spawnPopup();
    window.setInterval(() => {
      if (Math.random() < 0.3) spawnPopup();
    }, 45000);
  });
};
