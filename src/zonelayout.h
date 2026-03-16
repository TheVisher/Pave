#pragma once

#include <QHash>
#include <QRect>
#include <QSet>
#include <QString>
#include <QVector>

/// Defines a monitor's zone layout via split ratios.
///
/// A layout has:
/// - One optional vertical split (left/right columns)
/// - One optional horizontal split per column (top/bottom within a column)
///
/// Zone IDs: "L", "R", "L.T", "L.B", "R.T", "R.B"
/// When no vertical split: single zone "root" (almost-maximized state)
class ZoneLayout
{
public:
    ZoneLayout();

    /// Compute zone rects from the layout ratios, screen geometry, and gap size.
    /// Returns a map of zone ID -> rect.
    /// If activeZones is non-empty, only those zones get space; inactive zones
    /// are omitted and their space is absorbed by active neighbors (expand-to-fill).
    QHash<QString, QRect> computeRects(const QRect &screen, int gap,
                                        const QSet<QString> &activeZones = {}) const;

    /// Step the vertical split ratio up (right) or down (left).
    /// If no split exists, creates one at 1/2.
    /// If at the edge and stepping outward, collapses the shrinking zone (dormant).
    bool stepVerticalRatio(bool increase);

    /// Step the horizontal split ratio in a column.
    /// If no split exists in that column, creates one at 1/2.
    /// @param leftColumn  true = left column, false = right column
    /// @param increase    true = move split down, false = move split up
    bool stepHorizontalRatio(bool leftColumn, bool increase);

    /// Whether a vertical split is active (not dormant/collapsed).
    bool hasVerticalSplit() const { return m_hasVerticalSplit; }

    /// Whether a column has a horizontal split.
    bool hasHorizontalSplit(bool leftColumn) const;

    double verticalRatio() const { return m_verticalRatio; }
    void setVerticalRatio(double ratio) { m_verticalRatio = ratio; }
    double horizontalRatio(bool leftColumn) const;

    /// Whether the vertical ratio is at the maximum step (L at 3/4).
    bool isAtMaxRatio() const;
    /// Whether the vertical ratio is at the minimum step (L at 1/4).
    bool isAtMinRatio() const;

    /// Collapse a column's horizontal split back to a single zone.
    void collapseHorizontalSplit(bool leftColumn);

    /// Get the list of currently active zone IDs.
    QVector<QString> activeZoneIds() const;

    /// Find the adjacent zone in a direction. Returns empty string if none.
    QString adjacentZone(const QString &fromZone, const QString &direction) const;

private:
    bool m_hasVerticalSplit = false;
    double m_verticalRatio = 0.5;  // Left column fraction of total width

    bool m_hasLeftHSplit = false;
    double m_leftHRatio = 0.5;     // Top fraction of left column height

    bool m_hasRightHSplit = false;
    double m_rightHRatio = 0.5;    // Top fraction of right column height
};
