/// Recover the shared release tag from Apple's three-component bundle version.
func irisReleaseVersion(marketingVersion: String, buildVersion: String?) -> String {
    guard let buildVersion, let code = Int(buildVersion), code >= 0 else {
        return marketingVersion
    }
    let revision = code % 100
    if code >= 2_026_090_801 {
        let year = code / 1_000_000
        let month = code / 10_000 % 100
        let day = code / 100 % 100
        // Only decode the new format when both bundle fields agree.
        if marketingVersion == "\(year).\(month).\(day * 100 + revision)" {
            let date = "\(year).\(month).\(day)"
            return revision > 0 ? "\(date).\(revision)" : date
        }
    }
    return revision > 0 ? "\(marketingVersion).\(revision)" : marketingVersion
}
