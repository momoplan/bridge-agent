import officialEnvironment from "../../config/official-environment.json";
import type { ComputerAction } from "./types";

export const DEFAULT_INLINE_LIMIT_BYTES = 256 * 1024;

export const SHELL_SCHEMA = {
  type: "object",
  required: ["command"],
  properties: {
    command: {
      description:
        'Command argv array for direct execution. On Windows, run shell built-ins or PATH lookup through cmd /C, for example ["cmd", "/C", "where", "wechat-decrypt"].',
      type: "array",
      items: { type: "string" },
      minItems: 1
    },
    cwd: { type: "string" },
    env: {
      type: "object",
      additionalProperties: { type: "string" }
    }
  }
};

export const HTTP_SCHEMA = {
  type: "object",
  additionalProperties: true
};

export const EMPTY_OBJECT_SCHEMA = {
  type: "object",
  additionalProperties: false,
  properties: {}
};

export const COMPUTER_MOUSE_SCHEMA = {
  type: "object",
  required: ["x", "y"],
  properties: {
    x: { type: "number" },
    y: { type: "number" },
    button: {
      type: "string",
      enum: ["left", "middle", "right"]
    },
    keys: {
      type: "array",
      items: { type: "string" }
    }
  }
};

export const COMPUTER_SCROLL_SCHEMA = {
  type: "object",
  required: ["x", "y"],
  properties: {
    x: { type: "number" },
    y: { type: "number" },
    scroll_x: { type: "integer" },
    scroll_y: { type: "integer" },
    scrollX: { type: "integer" },
    scrollY: { type: "integer" },
    keys: {
      type: "array",
      items: { type: "string" }
    }
  }
};

export const COMPUTER_TYPE_SCHEMA = {
  type: "object",
  required: ["text"],
  properties: {
    text: { type: "string" }
  }
};

export const COMPUTER_WAIT_SCHEMA = {
  type: "object",
  properties: {
    ms: {
      type: "integer",
      minimum: 0
    }
  }
};

export const COMPUTER_KEYPRESS_SCHEMA = {
  type: "object",
  required: ["keys"],
  properties: {
    keys: {
      type: "array",
      items: { type: "string" },
      minItems: 1
    }
  }
};

export const COMPUTER_DRAG_SCHEMA = {
  type: "object",
  required: ["path"],
  properties: {
    path: {
      type: "array",
      minItems: 2,
      items: {
        type: "object",
        required: ["x", "y"],
        properties: {
          x: { type: "number" },
          y: { type: "number" }
        }
      }
    },
    keys: {
      type: "array",
      items: { type: "string" }
    }
  }
};

export const DEFAULT_PLATFORM_BASE_URL = officialEnvironment.apiBaseUrl;
export const DEFAULT_CONSOLE_BASE_URL = "https://console.baijimu.com";
export const DEFAULT_SAFE_COMMANDS = "echo, pwd, ls, git";
export const FULL_ACCESS_COMMAND = "*";
export const FULL_ACCESS_ROOT_DIR = "/";

export const COMPUTER_METHOD_PRESETS: Record<
  ComputerAction,
  { name: string; description: string; schema: unknown; label: string }
> = {
  screenshot: {
    name: "screenshot",
    description: "Capture the current desktop and return a PNG screenshot.",
    schema: EMPTY_OBJECT_SCHEMA,
    label: "截图"
  },
  click: {
    name: "click",
    description: "Click at a screen coordinate with an optional mouse button.",
    schema: COMPUTER_MOUSE_SCHEMA,
    label: "单击"
  },
  double_click: {
    name: "double_click",
    description: "Double-click at a screen coordinate.",
    schema: COMPUTER_MOUSE_SCHEMA,
    label: "双击"
  },
  scroll: {
    name: "scroll",
    description: "Scroll at a screen coordinate with horizontal and vertical deltas.",
    schema: COMPUTER_SCROLL_SCHEMA,
    label: "滚动"
  },
  type: {
    name: "type",
    description: "Type text into the currently focused app.",
    schema: COMPUTER_TYPE_SCHEMA,
    label: "输入文本"
  },
  wait: {
    name: "wait",
    description: "Pause briefly to let the desktop settle before the next screenshot.",
    schema: COMPUTER_WAIT_SCHEMA,
    label: "等待"
  },
  keypress: {
    name: "keypress",
    description: "Press one key or a key chord such as Command+L.",
    schema: COMPUTER_KEYPRESS_SCHEMA,
    label: "按键"
  },
  drag: {
    name: "drag",
    description: "Drag the pointer across a path of coordinates.",
    schema: COMPUTER_DRAG_SCHEMA,
    label: "拖拽"
  },
  move: {
    name: "move",
    description: "Move the pointer to a screen coordinate.",
    schema: COMPUTER_MOUSE_SCHEMA,
    label: "移动"
  }
};
