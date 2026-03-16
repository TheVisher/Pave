#include "zonelayout.h"

// Vertical: 1/4, 1/3, 1/2, 2/3, 3/4
static const QVector<double> V_STEPS = {0.25, 1.0/3.0, 0.5, 2.0/3.0, 0.75};

// Horizontal: 1/3, 1/2, 2/3
static const QVector<double> H_STEPS = {1.0/3.0, 0.5, 2.0/3.0};

/// Find the closest step index for a ratio in a step list.
static int findClosestStep(double ratio, const QVector<double> &steps)
{
    int best = 0;
    double bestDist = std::abs(steps[0] - ratio);
    for (int i = 1; i < steps.size(); ++i) {
        double dist = std::abs(steps[i] - ratio);
        if (dist < bestDist) {
            bestDist = dist;
            best = i;
        }
    }
    return best;
}

ZoneLayout::ZoneLayout() = default;

QHash<QString, QRect> ZoneLayout::computeRects(const QRect &screen, int gap,
                                                 const QSet<QString> &activeZones) const
{
    QHash<QString, QRect> result;
    int sx = screen.x();
    int sy = screen.y();
    int sw = screen.width();
    int sh = screen.height();

    if (!m_hasVerticalSplit) {
        result.insert(QStringLiteral("root"), QRect(
            sx + gap, sy + gap,
            sw - 2 * gap, sh - 2 * gap
        ));
        return result;
    }

    // Determine which columns are active (for expand-to-fill)
    bool leftActive = true;
    bool rightActive = true;

    if (!activeZones.isEmpty()) {
        leftActive = activeZones.contains(QStringLiteral("L"))
                  || activeZones.contains(QStringLiteral("L.T"))
                  || activeZones.contains(QStringLiteral("L.B"));
        rightActive = activeZones.contains(QStringLiteral("R"))
                   || activeZones.contains(QStringLiteral("R.T"))
                   || activeZones.contains(QStringLiteral("R.B"));

        if (!leftActive && !rightActive) {
            leftActive = true;
            rightActive = true;
        }
    }

    int halfGap = gap / 2;

    if (leftActive && !rightActive) {
        int leftX = sx + gap;
        int leftW = sw - 2 * gap;

        if (!m_hasLeftHSplit) {
            result.insert(QStringLiteral("L"), QRect(leftX, sy + gap, leftW, sh - 2 * gap));
        } else {
            int splitY = sy + static_cast<int>(sh * m_leftHRatio);
            result.insert(QStringLiteral("L.T"), QRect(
                leftX, sy + gap, leftW, splitY - sy - gap - halfGap
            ));
            result.insert(QStringLiteral("L.B"), QRect(
                leftX, splitY + halfGap, leftW, sy + sh - splitY - gap - halfGap
            ));
        }
        return result;
    }

    if (!leftActive && rightActive) {
        int rightX = sx + gap;
        int rightW = sw - 2 * gap;

        if (!m_hasRightHSplit) {
            result.insert(QStringLiteral("R"), QRect(rightX, sy + gap, rightW, sh - 2 * gap));
        } else {
            int splitY = sy + static_cast<int>(sh * m_rightHRatio);
            result.insert(QStringLiteral("R.T"), QRect(
                rightX, sy + gap, rightW, splitY - sy - gap - halfGap
            ));
            result.insert(QStringLiteral("R.B"), QRect(
                rightX, splitY + halfGap, rightW, sy + sh - splitY - gap - halfGap
            ));
        }
        return result;
    }

    // Both columns active — normal vertical split
    int splitX = sx + static_cast<int>(sw * m_verticalRatio);

    int leftX = sx + gap;
    int leftW = splitX - sx - gap - halfGap;
    int rightX = splitX + halfGap;
    int rightW = sx + sw - splitX - gap - halfGap;

    if (!m_hasLeftHSplit) {
        result.insert(QStringLiteral("L"), QRect(
            leftX, sy + gap, leftW, sh - 2 * gap
        ));
    } else {
        int splitY = sy + static_cast<int>(sh * m_leftHRatio);
        result.insert(QStringLiteral("L.T"), QRect(
            leftX, sy + gap, leftW, splitY - sy - gap - halfGap
        ));
        result.insert(QStringLiteral("L.B"), QRect(
            leftX, splitY + halfGap, leftW, sy + sh - splitY - gap - halfGap
        ));
    }

    if (!m_hasRightHSplit) {
        result.insert(QStringLiteral("R"), QRect(
            rightX, sy + gap, rightW, sh - 2 * gap
        ));
    } else {
        int splitY = sy + static_cast<int>(sh * m_rightHRatio);
        result.insert(QStringLiteral("R.T"), QRect(
            rightX, sy + gap, rightW, splitY - sy - gap - halfGap
        ));
        result.insert(QStringLiteral("R.B"), QRect(
            rightX, splitY + halfGap, rightW, sy + sh - splitY - gap - halfGap
        ));
    }

    return result;
}

bool ZoneLayout::stepVerticalRatio(bool increase)
{
    if (!m_hasVerticalSplit) {
        m_hasVerticalSplit = true;
        m_verticalRatio = 0.5;
        return true;
    }

    int idx = findClosestStep(m_verticalRatio, V_STEPS);

    if (increase) {
        if (idx + 1 < V_STEPS.size()) {
            m_verticalRatio = V_STEPS[idx + 1];
            return true;
        }
    } else {
        if (idx > 0) {
            m_verticalRatio = V_STEPS[idx - 1];
            return true;
        }
    }
    return false;  // At edge, clamped
}

bool ZoneLayout::isAtMaxRatio() const
{
    if (!m_hasVerticalSplit) return false;
    int idx = findClosestStep(m_verticalRatio, V_STEPS);
    return idx == V_STEPS.size() - 1;
}

bool ZoneLayout::isAtMinRatio() const
{
    if (!m_hasVerticalSplit) return false;
    int idx = findClosestStep(m_verticalRatio, V_STEPS);
    return idx == 0;
}

bool ZoneLayout::stepHorizontalRatio(bool leftColumn, bool increase)
{
    bool &hasSplit = leftColumn ? m_hasLeftHSplit : m_hasRightHSplit;
    double &ratio = leftColumn ? m_leftHRatio : m_rightHRatio;

    if (!hasSplit) {
        hasSplit = true;
        ratio = 0.5;
        return true;
    }

    int idx = findClosestStep(ratio, H_STEPS);

    if (increase) {
        if (idx + 1 < H_STEPS.size()) {
            ratio = H_STEPS[idx + 1];
            return true;
        }
    } else {
        if (idx > 0) {
            ratio = H_STEPS[idx - 1];
            return true;
        }
    }
    return false;  // At edge, clamped
}

void ZoneLayout::collapseHorizontalSplit(bool leftColumn)
{
    if (leftColumn) {
        m_hasLeftHSplit = false;
        m_leftHRatio = 0.5;
    } else {
        m_hasRightHSplit = false;
        m_rightHRatio = 0.5;
    }
}

bool ZoneLayout::hasHorizontalSplit(bool leftColumn) const
{
    return leftColumn ? m_hasLeftHSplit : m_hasRightHSplit;
}

double ZoneLayout::horizontalRatio(bool leftColumn) const
{
    return leftColumn ? m_leftHRatio : m_rightHRatio;
}

QVector<QString> ZoneLayout::activeZoneIds() const
{
    QVector<QString> ids;

    if (!m_hasVerticalSplit) {
        ids.append(QStringLiteral("root"));
        return ids;
    }

    if (m_hasLeftHSplit) {
        ids.append(QStringLiteral("L.T"));
        ids.append(QStringLiteral("L.B"));
    } else {
        ids.append(QStringLiteral("L"));
    }

    if (m_hasRightHSplit) {
        ids.append(QStringLiteral("R.T"));
        ids.append(QStringLiteral("R.B"));
    } else {
        ids.append(QStringLiteral("R"));
    }

    return ids;
}

QString ZoneLayout::adjacentZone(const QString &fromZone, const QString &direction) const
{
    if (direction == QLatin1String("left")) {
        if (fromZone == QLatin1String("R")) return m_hasLeftHSplit ? QStringLiteral("L.T") : QStringLiteral("L");
        if (fromZone == QLatin1String("R.T")) return m_hasLeftHSplit ? QStringLiteral("L.T") : QStringLiteral("L");
        if (fromZone == QLatin1String("R.B")) return m_hasLeftHSplit ? QStringLiteral("L.B") : QStringLiteral("L");
    }
    if (direction == QLatin1String("right")) {
        if (fromZone == QLatin1String("L")) return m_hasRightHSplit ? QStringLiteral("R.T") : QStringLiteral("R");
        if (fromZone == QLatin1String("L.T")) return m_hasRightHSplit ? QStringLiteral("R.T") : QStringLiteral("R");
        if (fromZone == QLatin1String("L.B")) return m_hasRightHSplit ? QStringLiteral("R.B") : QStringLiteral("R");
    }

    if (direction == QLatin1String("up")) {
        if (fromZone == QLatin1String("L.B")) return QStringLiteral("L.T");
        if (fromZone == QLatin1String("R.B")) return QStringLiteral("R.T");
    }
    if (direction == QLatin1String("down")) {
        if (fromZone == QLatin1String("L.T")) return QStringLiteral("L.B");
        if (fromZone == QLatin1String("R.T")) return QStringLiteral("R.B");
    }

    return QString();
}
