const siteUrl = process.env.HADRIS_SITE_URL || "https://hxyulin.github.io";
const baseUrl = process.env.HADRIS_BASE_URL || "/hadris/";

module.exports = {
  docs: [
    "index",
    "getting-started",
    "crates",
    {
      type: "category",
      label: "Concepts",
      items: [
        "concepts/features",
        "concepts/storage-model",
      ],
    },
    {
      type: "category",
      label: "Use cases",
      link: {type: "doc", id: "guides/index"},
      items: [
        "guides/detect-open-images",
        "guides/open-partitioned-fat",
        "guides/read-fat-image",
        "guides/read-partition-table",
        "guides/read-iso",
        "guides/read-udf",
        "guides/cpio-archives",
        "guides/build-initramfs",
        "guides/modify-fat",
        "guides/async-io",
        "guides/custom-io",
        "guides/no-std",
        "guides/embedded",
        "guides/validate-images",
      ],
    },
    {
      type: "category",
      label: "Filesystem creation",
      items: [
        "creation/iso",
        "creation/fat",
        "creation/udf",
      ],
    },
    "stability",
    "contributing",
    {
      type: "link",
      label: "API reference",
      href: `${siteUrl}${baseUrl}next/api/hadris/index.html`,
    },
  ],
};
