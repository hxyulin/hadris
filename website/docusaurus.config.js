// @ts-check

const config = {
  title: "Hadris",
  tagline: "The Rust storage stack",
  favicon: "img/favicon.svg",
  url: "https://hxyulin.github.io",
  baseUrl: "/hadris/",
  organizationName: "hxyulin",
  projectName: "hadris",
  onBrokenLinks: "throw",
  markdown: {
    hooks: {
      onBrokenMarkdownLinks: "warn",
    },
  },
  i18n: {
    defaultLocale: "en",
    locales: ["en"],
  },
  presets: [
    [
      "classic",
      {
        docs: {
          routeBasePath: "/",
          sidebarPath: require.resolve("./sidebars.js"),
          editUrl: "https://github.com/hxyulin/hadris/edit/main/website/",
        },
        blog: false,
        theme: {
          customCss: require.resolve("./src/css/custom.css"),
        },
      },
    ],
  ],
  themeConfig: {
    navbar: {
      title: "Hadris",
      items: [
        {to: "/getting-started", label: "Get started", position: "left"},
        {to: "/guides", label: "Use cases", position: "left"},
        {to: "/crates", label: "Crates", position: "left"},
        {
          href: "https://docs.rs/hadris",
          label: "API docs",
          position: "right",
        },
        {
          href: "https://github.com/hxyulin/hadris",
          label: "GitHub",
          position: "right",
        },
      ],
    },
    footer: {
      style: "dark",
      links: [
        {
          title: "Documentation",
          items: [
            {label: "Get started", to: "/getting-started"},
            {label: "Use cases", to: "/guides"},
            {label: "Migration guide", to: "/migration/v1-to-v2"},
          ],
        },
        {
          title: "Project",
          items: [
            {label: "API docs", href: "https://docs.rs/hadris"},
            {label: "Crates.io", href: "https://crates.io/crates/hadris"},
            {label: "GitHub", href: "https://github.com/hxyulin/hadris"},
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} Hadris contributors. MIT licensed.`,
    },
  },
};

module.exports = config;
