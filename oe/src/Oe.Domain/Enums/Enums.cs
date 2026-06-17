namespace Oe.Domain.Enums;

public enum UserRole        { Admin, Sales, Accounts, Warehouse }
public enum CustomerStatus  { Active, Inactive, Suspended }
public enum OrderStatus     { Draft, Submitted, Fulfilled, Cancelled }
public enum InvoiceStatus   { Draft, Issued, PartiallyPaid, Paid, Overdue, Voided, WrittenOff }
public enum PaymentStatus   { Pending, Cleared, Reversed }
