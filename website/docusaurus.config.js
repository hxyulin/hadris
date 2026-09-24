// @ts-check

const {themes: prismThemes} = require("prism-react-renderer");

const siteUrl = process.env.HADRIS_SITE_URL || "https://hxyulin.github.io";
const baseUrl = process.env.HADRIS_BASE_URL || "/hadris/";
const channel = process.env.HADRIS_DOCS_CHANNEL || "stable";
const isNext = channel === "next";
const stableUrl = process.env.HADRIS_STABLE_URL || `${siteUrl}${baseUrl}`;
const nextUrl = process.env.HADRIS_NEXT_URL || `${siteUrl}${baseUrl}next/`;
const apiDocsUrl = isNext ? `${siteUrl}${baseUrl}api/hadris/index.html` : "https://docs.rs/hadris";

const config = {
  title: "Hadris",
  tagline: "The Rust storage stack",
  favicon: "img/favicon.svg",
  url: siteUrl,
  baseUrl,
  organizationName: "hxyulin",
  projectName: "hadris",
  onBrokenLinks: "throw",
  markdown: {
    hooks: {
      onBrokenMarkdownLinks: "throw",
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
          editUrl: `https://github.com/hxyulin/hadris/edit/${isNext ? "next" : "main"}/website/`,
        },
        blog: false,
        theme: {
          customCss: require.resolve("./src/css/custom.css"),
        },
      },
    ],
  ],
  themeConfig: {
    ...(isNext && {
      announcementBar: {
        id: "next",
        content: `These docs follow the unreleased 3.0 API on the <code>next</code> branch. <a href="${stableUrl}">Docs for the released 2.x crates</a>.`,
        isCloseable: false,
      },
    }),
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
      additionalLanguages: ["bash", "rust", "toml"],
    },
    navbar: {
      title: "Hadris",
      items: [
        {to: "/getting-started", label: "Get started", position: "left"},
        {to: "/guides", label: "Use cases", position: "left"},
        {to: "/crates", label: "Crates", position: "left"},
        {
          href: apiDocsUrl,
          label: "API docs",
          position: "right",
        },
        {
          href: isNext ? stableUrl : nextUrl,
          label: isNext ? "2.x docs" : "3.0 preview",
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
          ],
        },
        {
          title: "Project",
          items: [
            {label: "API docs", href: apiDocsUrl},
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
