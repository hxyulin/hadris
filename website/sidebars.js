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
  ],
};
