module.exports = {
  docs: [
    "index",
    "getting-started",
    "crates",
    {
      type: "category",
      label: "Use cases",
      link: {type: "doc", id: "guides/index"},
      items: [
        "guides/read-fat-image",
        "guides/read-partition-table",
        "guides/read-and-create-iso",
        "guides/build-initramfs",
        "guides/no-std",
      ],
    },
    {
      type: "category",
      label: "Migration",
      items: ["migration/v1-to-v2"],
    },
    "release-candidate",
    "contributing",
  ],
};
